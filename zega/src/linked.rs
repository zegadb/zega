//! APS 39 references and the engine/Worker wire contract.
//!
//! Reference identifiers are opaque external keys, never internal numeric node
//! slots. Transport types live in the engine so source and subscriber use the
//! same version and diff representation.

use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fmt, str::FromStr, time::Duration};

pub const DEFAULT_COALESCE_WINDOW: Duration = Duration::from_secs(5);
pub const DEFAULT_PUSH_TIMEOUT: Duration = Duration::from_millis(300);
pub const MAILBOX_TTL: Duration = Duration::from_secs(30 * 24 * 60 * 60);
pub const MAX_CHECK_ITEMS: usize = 1000;

/// A stable `zega://graph/id` reference. No URL decoding or numeric conversion
/// is applied to either component, preserving the source's external identity.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Reference {
    graph: String,
    id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("expected zega://graph/id with nonempty graph and id and no query or fragment")]
pub struct InvalidReference;

impl Reference {
    pub fn new(graph: &str, id: &str) -> Result<Self, InvalidReference> {
        fn valid(component: &str) -> bool {
            !component.is_empty()
                && !component.chars().any(|c| {
                    c.is_whitespace() || c.is_control() || matches!(c, '/' | '?' | '#' | '\\')
                })
        }
        if !valid(graph) || !valid(id) {
            return Err(InvalidReference);
        }
        Ok(Self {
            graph: graph.into(),
            id: id.into(),
        })
    }

    pub fn graph(&self) -> &str {
        &self.graph
    }
    pub fn id(&self) -> &str {
        &self.id
    }
}

impl FromStr for Reference {
    type Err = InvalidReference;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (graph, id) = value
            .strip_prefix("zega://")
            .and_then(|s| s.split_once('/'))
            .ok_or(InvalidReference)?;
        Self::new(graph, id)
    }
}
impl TryFrom<String> for Reference {
    type Error = InvalidReference;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}
impl From<Reference> for String {
    fn from(value: Reference) -> Self {
        value.to_string()
    }
}
impl fmt::Display for Reference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "zega://{}/{}", self.graph, self.id)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Diff {
    pub source: String,
    pub graph_version: u64,
    pub changes: Vec<NodeChange>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NodeChange {
    pub id: String,
    pub version: u64,
    pub kind: NodeKind,
    #[serde(flatten)]
    pub change: Change,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeKind {
    Node,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "lowercase")]
pub enum Change {
    Upsert {
        fields: BTreeMap<String, serde_json::Value>,
        rels: RelationshipChanges,
    },
    Delete,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelationshipChanges {
    pub add: Vec<Relationship>,
    pub remove: Vec<Relationship>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Relationship {
    #[serde(rename = "type")]
    pub kind: String,
    pub to: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SubscribeRequest {
    pub subscriber: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    pub ids: Vec<String>,
    pub lease_secs: u64,
}

// Deliberately no Debug: tokens must not enter diagnostic output.
#[derive(Clone, Serialize, Deserialize)]
pub struct SubscribeResponse {
    pub mailbox_token: String,
    pub graph_version: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CheckRequest {
    pub items: Vec<(String, u64)>,
}

impl CheckRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.items.len() > MAX_CHECK_ITEMS {
            Err("/sync/check accepts at most 1000 items")
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CheckResponse {
    pub stale: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MailboxPage {
    pub items: Vec<MailboxItem>,
    pub cursor: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MailboxItem {
    pub key: String,
    pub diff: Diff,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn references_preserve_external_identity() {
        for text in [
            "zega://earth/Q2096",
            "zega://fan/00042",
            "zega://graph/a%20b",
        ] {
            let reference: Reference = text.parse().unwrap();
            assert_eq!(reference.to_string(), text);
            let bytes = bincode::serialize(&reference).unwrap();
            assert_eq!(
                bincode::deserialize::<Reference>(&bytes).unwrap(),
                reference
            );
            assert_eq!(serde_json::to_value(&reference).unwrap(), json!(text));
        }
        for text in [
            "earth/Q2096",
            "https://earth/Q2096",
            "zega:///Q2096",
            "zega://earth/",
            "zega://earth/Q1/Q2",
            "zega://earth/Q1?q=1",
            "zega://earth/Q1#x",
            "zega://earth/Q1\n",
        ] {
            assert!(text.parse::<Reference>().is_err(), "{text:?}");
            assert!(serde_json::from_value::<Reference>(json!(text)).is_err());
        }
    }

    #[test]
    fn worker_contract_round_trip_preserves_versions_and_field_diffs() {
        let wire = json!({"source":"earth", "graph_version":u64::MAX,
        "changes":[
            {"id":"Q2096", "version":42, "kind":"node", "op":"upsert",
             "fields":{"name":"Edmonton"},
             "rels":{"add":[{"type":"locatedIn","to":"Q1951"}],"remove":[]}},
            {"id":"Q9999","version":7,"kind":"node","op":"delete"}
        ]});
        let diff: Diff = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(diff.graph_version, u64::MAX);
        assert_eq!(serde_json::to_value(diff).unwrap(), wire);
    }

    #[test]
    fn check_batch_has_an_inclusive_thousand_item_limit() {
        let mut request = CheckRequest {
            items: vec![("Q1".into(), 1); 1000],
        };
        assert!(request.validate().is_ok());
        request.items.push(("Q2".into(), 2));
        assert!(request.validate().is_err());
    }
}
