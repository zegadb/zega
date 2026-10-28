//! APS 52: deliveries. A source (pg_zega's journal, say) sends a document
//! under an identity, `(source_id, txid)`. [`Zega::deliver`] applies it as
//! one transaction and keeps a [`Receipt`] in the same WAL entry as its
//! writes: the writes and the receipt are durable together or not at all.
//! The same identity and document again returns the original receipt and
//! writes nothing; the same identity with a different document is refused.
//!
//! A delivery can name types to flush first ([`Flush`]): pg_zega's
//! `zega.import` in journal mode empties a type this way, then loads it again
//! in the same delivery.
//!
//! A source that will never send its earlier deliveries again says so with
//! [`Payload::forget_earlier`]: the delivery then keeps its own receipt and
//! drops the source's others, in the same WAL entry. pg_zega's worker does
//! this on every delivery, so a graph keeps one receipt per database.
//!
//! Receipts are graph state: replayed from the WAL, carried by checkpoints
//! (the `.graph` file's `RCPT` section), and copied wherever the WAL goes.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{Result, Zega, ZegaError};

/// What a delivery did, kept with the graph.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Receipt {
    pub source_id: String,
    pub txid: String,
    /// SHA-256 of the delivered schema and document ([`payload_sha256`]).
    pub payload_sha256: [u8; 32],
    /// The document's result, as JSON text.
    pub result: String,
}

impl Receipt {
    pub(crate) fn key(&self) -> (String, String) {
        (self.source_id.clone(), self.txid.clone())
    }
}

/// What a delivery deletes before its schema and document: every
/// relationship and node when `all`, else the relationships of each type in
/// `relationships` and the nodes with a label in `labels` (with every
/// relationship they still have).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Flush {
    #[serde(default)]
    pub all: bool,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub relationships: Vec<String>,
}

impl Flush {
    pub fn is_empty(&self) -> bool {
        !self.all && self.labels.is_empty() && self.relationships.is_empty()
    }
}

/// What one delivery carries: applied in this order, as one transaction.
#[derive(Clone, Debug, Default)]
pub struct Payload<'a> {
    pub flush: Flush,
    /// Stored as the graph's schema (as [`Zega::set_schema`]).
    pub schema: Option<&'a str>,
    /// A `.zql` document: schema, then its blocks.
    pub document: &'a str,
    /// Drop this source's other receipts: it will never send an earlier
    /// delivery again. Not part of the payload's digest.
    pub forget_earlier: bool,
}

impl<'a> Payload<'a> {
    /// A payload of `document` alone.
    pub fn document(document: &'a str) -> Self {
        Payload { document, ..Payload::default() }
    }
}

/// The digest a receipt binds: domain-separated and length-prefixed. A
/// delivery without a flush hashes as before flushes existed: the domain,
/// the document, then the schema if there is one. With a flush, a v2 domain
/// marks whether a schema follows, and the flush comes last.
pub fn payload_sha256(payload: &Payload<'_>) -> [u8; 32] {
    let flush = (!payload.flush.is_empty()).then(|| serde_json::to_vec(&payload.flush).expect("a flush serializes"));
    let schema = payload.schema.map(str::as_bytes);
    let mut parts = vec![];
    match &flush {
        None => parts.extend([Some(b"zega-delivery-v1".as_slice()), Some(payload.document.as_bytes()), schema]),
        Some(flush) => parts.extend([
            Some(b"zega-delivery-v2".as_slice()),
            Some(payload.document.as_bytes()),
            Some(if schema.is_some() { b"schema".as_slice() } else { b"no schema" }),
            schema,
            Some(flush.as_slice()),
        ]),
    }
    let mut hash = Sha256::new();
    for part in parts.into_iter().flatten() {
        hash.update((part.len() as u64).to_le_bytes());
        hash.update(part);
    }
    hash.finalize().into()
}

fn refused(code: &'static str, message: impl Into<String>) -> ZegaError {
    ZegaError::Delivery { code, message: message.into() }
}

/// 1 to 128 bytes of `A-Z a-z 0-9 - _ . :`.
fn check_identity(source_id: &str, txid: &str) -> Result<()> {
    for value in [source_id, txid] {
        if value.is_empty()
            || value.len() > 128
            || !value.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.:".contains(&b))
        {
            return Err(refused("invalid_identity", "source_id and txid are 1 to 128 bytes of A-Z a-z 0-9 - _ . :"));
        }
    }
    Ok(())
}

impl Zega {
    /// Apply `payload` once for `(source_id, txid)`: its flush, its schema,
    /// then its document. See the module documentation.
    pub fn deliver(&self, source_id: &str, txid: &str, payload: &Payload<'_>) -> Result<Receipt> {
        check_identity(source_id, txid)?;
        let payload_sha256 = payload_sha256(payload);
        self.transaction(|tx| {
            if let Some(existing) = tx.receipt(source_id, txid) {
                return if existing.payload_sha256 == payload_sha256 {
                    Ok(existing)
                } else {
                    Err(refused("delivery_conflict", format!("{source_id} {txid} was delivered with a different document")))
                };
            }
            if !payload.flush.is_empty() {
                tx.flush(&payload.flush)?;
            }
            if let Some(schema) = payload.schema {
                tx.schema(schema)?;
            }
            let result = tx.document(payload.document)?;
            let receipt = Receipt {
                source_id: source_id.into(),
                txid: txid.into(),
                payload_sha256,
                result: result.to_string(),
            };
            tx.record_receipt(receipt.clone());
            if payload.forget_earlier {
                tx.forget_receipts(source_id, txid);
            }
            Ok(receipt)
        })
    }

    /// The receipt of `(source_id, txid)`, if it was delivered.
    pub fn receipt(&self, source_id: &str, txid: &str) -> Result<Option<Receipt>> {
        Ok(self.lock_graph()?.receipts.get(&(source_id.to_string(), txid.to_string())).cloned())
    }
}
