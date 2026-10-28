//! APS 52: deliveries. A source (pg_zega's journal, say) sends a document
//! under an identity, `(source_id, txid)`. [`Zega::deliver`] applies it as
//! one transaction and keeps a [`Receipt`] in the same WAL entry as its
//! writes: the writes and the receipt are durable together or not at all.
//! The same identity and document again returns the original receipt and
//! writes nothing; the same identity with a different document is refused.
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

/// The digest a receipt binds: domain-separated and length-prefixed. A
/// delivery without a schema hashes the domain and the document alone.
pub fn payload_sha256(schema: Option<&str>, document: &str) -> [u8; 32] {
    let mut hash = Sha256::new();
    let parts = [Some(b"zega-delivery-v1".as_slice()), Some(document.as_bytes()), schema.map(str::as_bytes)];
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
    /// Apply `document` (a `.zql` document: schema, then its blocks) once for
    /// `(source_id, txid)`, after storing `schema` as the graph's schema when
    /// one is given (as [`Zega::set_schema`]). See the module documentation.
    pub fn deliver(&self, source_id: &str, txid: &str, schema: Option<&str>, document: &str) -> Result<Receipt> {
        check_identity(source_id, txid)?;
        let payload_sha256 = payload_sha256(schema, document);
        self.transaction(|tx| {
            if let Some(existing) = tx.receipt(source_id, txid) {
                return if existing.payload_sha256 == payload_sha256 {
                    Ok(existing)
                } else {
                    Err(refused("delivery_conflict", format!("{source_id} {txid} was delivered with a different document")))
                };
            }
            if let Some(schema) = schema {
                tx.schema(schema)?;
            }
            let result = tx.document(document)?;
            let receipt = Receipt {
                source_id: source_id.into(),
                txid: txid.into(),
                payload_sha256,
                result: result.to_string(),
            };
            tx.record_receipt(receipt.clone());
            Ok(receipt)
        })
    }

    /// The receipt of `(source_id, txid)`, if it was delivered.
    pub fn receipt(&self, source_id: &str, txid: &str) -> Result<Option<Receipt>> {
        Ok(self.lock_graph()?.receipts.get(&(source_id.to_string(), txid.to_string())).cloned())
    }
}
