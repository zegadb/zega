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

use crate::{
    graph::{Graph, NodeId},
    wal::Operation,
    Value, Zega, ZegaError,
};
use std::collections::HashMap;

/// Engine-owned sync metadata, checkpointed alongside the graph. The write
/// path touches only the changed nodes; it never walks subscriber leases.
#[derive(Clone, Default, Serialize, Deserialize)]
pub(crate) struct Store {
    #[serde(default)]
    pub enabled: bool,
    pub graph_version: u64,
    pub identities: HashMap<String, NodeId>,
    pub external: HashMap<NodeId, String>,
    pub versions: HashMap<String, u64>,
    pub history: Vec<Record>,
    pub leases: HashMap<String, std::sync::Arc<HashMap<String, Lease>>>,
    #[serde(skip)]
    pub hook: Option<std::sync::Arc<dyn Fn() + Send + Sync>>,
    pub mirrors: HashMap<NodeId, Mirror>,
}
impl Store {
    pub fn active(&self) -> bool {
        self.enabled
            || self.graph_version != 0
            || !self.leases.is_empty()
            || !self.mirrors.is_empty()
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Record {
    pub graph_version: u64,
    pub committed_at: u64,
    pub changes: Vec<NodeChange>,
    pub(crate) identities: Vec<(String, NodeId)>,
}
impl Record {
    /// A transport batch omits internal identity-slot bookkeeping.
    pub fn scoped(graph_version: u64, committed_at: u64, changes: Vec<NodeChange>) -> Self {
        Self {
            graph_version,
            committed_at,
            changes,
            identities: Vec::new(),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Lease {
    pub subscriber: String,
    pub endpoint: Option<String>,
    pub expires_at: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Mirror {
    pub source: String,
    pub id: String,
    pub version: u64,
    pub source_gone: bool,
    /// Source relationships only. Local edges are not removed during sync.
    pub relationships: Vec<u64>,
    pub stub: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeSnapshot {
    pub id: String,
    pub version: u64,
    pub labels: Vec<String>,
    pub fields: BTreeMap<String, serde_json::Value>,
    /// Native shapes absent from JSON (notably vectors versus numeric lists).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub types: BTreeMap<String, ValueShape>,
    pub rels: Vec<Relationship>,
    pub stubs: Vec<Stub>,
    pub source_gone: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Stub {
    pub id: String,
    pub labels: Vec<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type")]
pub(crate) enum Metadata {
    Enable,
    Commit { record: Record },
    Subscribe { ids: Vec<String>, lease: Lease },
    Mirror { node: NodeId, mirror: Mirror },
}
pub(crate) fn operation(meta: &Metadata) -> Operation {
    Operation::Linked {
        bytes: serde_json::to_vec(meta).expect("serializable linked metadata"),
    }
}
pub(crate) fn replay(graph: &mut Graph, bytes: &[u8]) -> crate::Result<()> {
    let meta: Metadata =
        serde_json::from_slice(bytes).map_err(|e| ZegaError::Execution(e.to_string()))?;
    apply_metadata(graph, meta);
    Ok(())
}
pub(crate) fn apply_metadata(graph: &mut Graph, meta: Metadata) {
    let store = &mut graph.linked;
    match meta {
        Metadata::Enable => store.enabled = true,
        Metadata::Commit { record } => {
            store.graph_version = record.graph_version;
            for (external, node) in &record.identities {
                store.identities.insert(external.clone(), *node);
                store.external.insert(*node, external.clone());
            }
            for change in &record.changes {
                store.versions.insert(change.id.clone(), change.version);
            }
            store.history.push(record);
            if let Some(hook) = &store.hook {
                hook();
            }
        }
        Metadata::Subscribe { ids, lease } => {
            for id in ids {
                std::sync::Arc::make_mut(store.leases.entry(id).or_default())
                    .insert(lease.subscriber.clone(), lease.clone());
            }
        }
        Metadata::Mirror { node, mirror } => {
            store.enabled = true;
            store.mirrors.insert(node, mirror);
        }
    }
}
pub fn now_secs() -> u64 {
    crate::history::now().max(0) as u64
}

fn failure(message: impl Into<String>) -> ZegaError {
    ZegaError::Execution(message.into())
}

/// Produce field-level changes from the journal's WAL operations, not a graph
/// scan or a second history system. Called before append, published after it.
pub(crate) fn record(
    graph: &Graph,
    ops: &[Operation],
    removed: &[crate::graph::Relationship],
) -> crate::Result<Option<Record>> {
    if ops.is_empty() {
        return Ok(None);
    }
    let version = graph
        .linked
        .graph_version
        .checked_add(1)
        .ok_or_else(|| failure("graph version exhausted"))?;
    let mut record = Record {
        graph_version: version,
        committed_at: now_secs(),
        changes: Vec::new(),
        identities: Vec::new(),
    };
    let mut ids = HashMap::<NodeId, String>::new();
    let mut inserted = std::collections::HashSet::new();
    for op in ops {
        if let Operation::InsertNode { id, props, .. } | Operation::InsertNodeAt { id, props, .. } =
            op
        {
            // An explicitly supplied string id is the graph's stable external
            // identity (earth uses QIDs). Otherwise allocate an opaque key.
            let external = props
                .get("id")
                .and_then(Value::as_string)
                .map(str::to_owned)
                .unwrap_or_else(|| format!("n{version}-{id}"));
            Reference::new("graph", &external).map_err(|e| failure(e.to_string()))?;
            if graph
                .linked
                .identities
                .get(&external)
                .is_some_and(|old| old != id)
                || !inserted.insert(external.clone())
            {
                return Err(failure(format!(
                    "stable external id {external} already exists; update its node instead"
                )));
            }
            ids.insert(*id, external.clone());
            record.identities.push((external, *id));
        }
    }
    let key = |id: NodeId| {
        ids.get(&id)
            .or_else(|| graph.linked.external.get(&id))
            .cloned()
    };
    let mut changes = BTreeMap::<String, Change>::new();
    for op in ops {
        match op {
            Operation::InsertNode { id, props, .. }
            | Operation::InsertNodeAt { id, props, .. }
            | Operation::UpdateNode { id, props }
            | Operation::UpdateNodeAt { id, props, .. } => {
                if graph.linked.mirrors.contains_key(id) {
                    continue;
                }
                if let Some(external) = key(*id) {
                    if let Some(new) = props.get("id").and_then(Value::as_string) {
                        if new != external {
                            return Err(failure("a node's stable external id cannot be changed"));
                        }
                    }
                    let entry = changes.entry(external).or_insert_with(empty_change);
                    if let Change::Upsert { fields, .. } = entry {
                        fields.extend(props.iter().map(|(k, v)| {
                            // A backdated HIST write may leave a newer
                            // current fact in place. Mirrors copy that
                            // current fact, not the historical input.
                            let current = graph
                                .get_node(*id)
                                .and_then(|node| node.prop(k))
                                .unwrap_or(v);
                            (k.clone(), crate::v2::value_to_json(current))
                        }));
                    }
                }
            }
            Operation::DeleteNode { id } => {
                if let Some(id) = key(*id) {
                    changes.insert(id, Change::Delete);
                }
            }
            Operation::InsertRel { from, to, kind, .. } => {
                add_rel_change(&mut changes, key(*from), key(*to), kind, true)
            }
            Operation::InsertRelAt { rel, .. } => {
                add_rel_change(&mut changes, key(rel.from), key(rel.to), &rel.kind, true)
            }
            Operation::EndRelAt { rel, .. } => {
                add_rel_change(&mut changes, key(rel.from), key(rel.to), &rel.kind, false)
            }
            Operation::DeleteRel { id } => {
                if let Some(rel) = removed.iter().find(|r| r.id == *id) {
                    add_rel_change(&mut changes, key(rel.from), key(rel.to), &rel.kind, false);
                }
            }
            _ => {}
        }
    }
    record.changes = changes
        .into_iter()
        .map(|(id, change)| {
            let version = graph
                .linked
                .versions
                .get(&id)
                .copied()
                .unwrap_or(0)
                .checked_add(1)
                .ok_or_else(|| failure("node version exhausted"))?;
            Ok(NodeChange {
                id,
                version,
                kind: NodeKind::Node,
                change,
            })
        })
        .collect::<crate::Result<_>>()?;
    Ok(Some(record))
}
fn empty_change() -> Change {
    Change::Upsert {
        fields: BTreeMap::new(),
        rels: RelationshipChanges::default(),
    }
}
fn add_rel_change(
    changes: &mut BTreeMap<String, Change>,
    from: Option<String>,
    to: Option<String>,
    kind: &str,
    add: bool,
) {
    let (Some(from), Some(to)) = (from, to) else {
        return;
    };
    if let Change::Upsert { rels, .. } = changes.entry(from).or_insert_with(empty_change) {
        let rel = Relationship {
            kind: kind.into(),
            to,
        };
        let (insert, remove) = if add {
            (&mut rels.add, &mut rels.remove)
        } else {
            (&mut rels.remove, &mut rels.add)
        };
        remove.retain(|r| r != &rel);
        if !insert.contains(&rel) {
            insert.push(rel);
        }
    }
}

impl Zega {
    pub fn graph_version(&self) -> crate::Result<u64> {
        Ok(self.lock_graph()?.linked.graph_version)
    }
    pub fn changes_since(&self, version: u64) -> crate::Result<Vec<Record>> {
        let graph = self.lock_graph()?;
        // Binary search keeps the asynchronous worker independent of history size.
        let start = graph
            .linked
            .history
            .partition_point(|r| r.graph_version <= version);
        Ok(graph.linked.history[start..].to_vec())
    }
    pub fn sync_check(&self, request: &CheckRequest) -> crate::Result<CheckResponse> {
        request.validate().map_err(failure)?;
        let graph = self.lock_graph()?;
        Ok(CheckResponse {
            stale: request
                .items
                .iter()
                .filter(|(id, v)| graph.linked.versions.get(id) != Some(v))
                .map(|(id, _)| id.clone())
                .collect(),
        })
    }
    pub fn subscribe(&self, request: &SubscribeRequest) -> crate::Result<u64> {
        if request.ids.len() > MAX_CHECK_ITEMS
            || request.lease_secs == 0
            || request.lease_secs > MAILBOX_TTL.as_secs()
        {
            return Err(failure(
                "subscription requires <=1000 ids and a lease of 1..2592000 seconds",
            ));
        }
        Reference::new("subscriber", &request.subscriber).map_err(|e| failure(e.to_string()))?;
        self.prepare_linked()?;
        let mut graph = self.lock_graph()?;
        if request
            .ids
            .iter()
            .any(|id| !graph.linked.identities.contains_key(id))
        {
            return Err(failure("unknown subscription id"));
        }
        let meta = Metadata::Subscribe {
            ids: request.ids.clone(),
            lease: Lease {
                subscriber: request.subscriber.clone(),
                endpoint: request.endpoint.clone(),
                expires_at: now_secs() + request.lease_secs,
            },
        };
        self.wal.append(&operation(&meta))?;
        apply_metadata(&mut graph, meta);
        Ok(graph.linked.graph_version)
    }
    /// Install a nonblocking wakeup only; subscriber work belongs to the host.
    pub fn on_commit(&self, hook: std::sync::Arc<dyn Fn() + Send + Sync>) -> crate::Result<()> {
        self.lock_graph()?.linked.hook = Some(hook);
        Ok(())
    }
    pub fn subscribers(
        &self,
        ids: &[String],
    ) -> crate::Result<HashMap<String, std::sync::Arc<HashMap<String, Lease>>>> {
        let graph = self.lock_graph()?;
        Ok(ids
            .iter()
            .filter_map(|id| {
                graph
                    .linked
                    .leases
                    .get(id)
                    .map(|leases| (id.clone(), leases.clone()))
            })
            .collect())
    }
    pub fn sync_node(&self, id: &str) -> crate::Result<NodeSnapshot> {
        self.prepare_linked()?;
        let graph = self.lock_graph()?;
        let node_id = *graph
            .linked
            .identities
            .get(id)
            .ok_or_else(|| failure("unknown external id"))?;
        let version = graph.linked.versions.get(id).copied().unwrap_or(0);
        let Some(node) = graph.get_node(node_id).filter(|_| {
            graph
                .linked
                .external
                .get(&node_id)
                .is_some_and(|external| external == id)
        }) else {
            return Ok(NodeSnapshot {
                id: id.into(),
                version,
                labels: Vec::new(),
                fields: BTreeMap::new(),
                types: BTreeMap::new(),
                rels: Vec::new(),
                stubs: Vec::new(),
                source_gone: true,
            });
        };
        let mut rels = Vec::new();
        let mut stubs = BTreeMap::new();
        for rel_id in graph.node_relationship_ids(node_id) {
            if let Some(rel) = graph.get_relationship(rel_id) {
                if rel.from != node_id {
                    continue;
                }
                if let Some(target) = graph.linked.external.get(&rel.to) {
                    rels.push(Relationship {
                        kind: rel.kind.into(),
                        to: target.clone(),
                    });
                    if let Some(node) = graph.get_node(rel.to) {
                        stubs.insert(
                            target.clone(),
                            Stub {
                                id: target.clone(),
                                labels: node.labels().map(str::to_owned).collect(),
                            },
                        );
                    }
                }
            }
        }
        Ok(NodeSnapshot {
            id: id.into(),
            version,
            labels: node.labels().map(str::to_owned).collect(),
            fields: node
                .props()
                .map(|(k, v)| (k.into(), crate::v2::value_to_json(v)))
                .collect(),
            types: node
                .props()
                .filter_map(|(k, v)| value_shape(v).map(|shape| (k.into(), shape)))
                .collect(),
            rels,
            stubs: stubs.into_values().collect(),
            source_gone: false,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ValueShape {
    Point,
    Vector(crate::vector::Metric),
    List(Vec<Option<ValueShape>>),
    Map(BTreeMap<String, ValueShape>),
}
fn value_shape(value: &Value) -> Option<ValueShape> {
    match value {
        Value::Point(_) => Some(ValueShape::Point),
        Value::Vector(v) => Some(ValueShape::Vector(v.metric)),
        Value::List(values) => {
            let types: Vec<_> = values.iter().map(value_shape).collect();
            types
                .iter()
                .any(Option::is_some)
                .then_some(ValueShape::List(types))
        }
        Value::Map(values) => {
            let types: BTreeMap<_, _> = values
                .iter()
                .filter_map(|(k, v)| value_shape(v).map(|shape| (k.clone(), shape)))
                .collect();
            (!types.is_empty()).then_some(ValueShape::Map(types))
        }
        _ => None,
    }
}
fn typed_json(value: &serde_json::Value, shape: Option<&ValueShape>) -> crate::Result<Value> {
    if value.is_null() {
        return Ok(Value::Null);
    }
    Ok(match shape {
        Some(ValueShape::Point) => {
            Value::Point(crate::location::Point::from_json(value).map_err(failure)?)
        }
        Some(ValueShape::Vector(metric)) => Value::Vector(Box::new(
            crate::vector::Vector::from_json(value, *metric).map_err(failure)?,
        )),
        Some(ValueShape::List(types)) => Value::List(
            value
                .as_array()
                .ok_or_else(|| failure("expected a list"))?
                .iter()
                .enumerate()
                .map(|(i, v)| typed_json(v, types.get(i).and_then(Option::as_ref)))
                .collect::<crate::Result<_>>()?,
        ),
        Some(ValueShape::Map(types)) => Value::Map(Box::new(
            value
                .as_object()
                .ok_or_else(|| failure("expected a map"))?
                .iter()
                .map(|(k, v)| Ok((k.clone(), typed_json(v, types.get(k))?)))
                .collect::<crate::Result<_>>()?,
        )),
        None => from_json(value),
    })
}
fn from_json(value: &serde_json::Value) -> Value {
    use serde_json::Value as J;
    match value {
        J::Null => Value::Null,
        J::Bool(v) => Value::Bool(*v),
        J::String(v) => Value::from(v.clone()),
        J::Number(v) => v
            .as_i64()
            .map(Value::Int)
            .unwrap_or_else(|| Value::from_f64(v.as_f64().unwrap_or_default())),
        J::Array(v) => Value::List(v.iter().map(from_json).collect()),
        J::Object(v) => Value::Map(Box::new(
            v.iter().map(|(k, v)| (k.clone(), from_json(v))).collect(),
        )),
    }
}
fn mirror_node(graph: &Graph, source: &str, id: &str) -> Option<NodeId> {
    graph
        .linked
        .mirrors
        .iter()
        .find(|(_, m)| m.source == source && m.id == id)
        .map(|(id, _)| *id)
}
impl Zega {
    /// Install the result of an explicit one-hop fetch. All node facts, stubs,
    /// source edges and provenance are one WAL statement. Queries stay local.
    pub fn link(&self, reference: &Reference, snapshot: &NodeSnapshot) -> crate::Result<NodeId> {
        if reference.id() != snapshot.id {
            return Err(failure("source returned a different external id"));
        }
        let mut graph = self.lock_graph()?;
        let mut next = graph.next_ids();
        let node = mirror_node(&graph, reference.graph(), reference.id()).unwrap_or_else(|| {
            let n = next.0;
            next.0 += 1;
            n
        });
        if graph
            .linked
            .mirrors
            .get(&node)
            .is_some_and(|m| !m.stub && m.version >= snapshot.version)
        {
            return Ok(node);
        }
        let mut ops = Vec::new();
        if !snapshot.source_gone {
            ops.push(Operation::InsertNode {
                id: node,
                labels: snapshot.labels.clone(),
                props: snapshot
                    .fields
                    .iter()
                    .map(|(k, v)| Ok((k.clone(), typed_json(v, snapshot.types.get(k))?)))
                    .collect::<crate::Result<_>>()?,
            });
        } else if graph.get_node(node).is_none() {
            ops.push(Operation::InsertNode {
                id: node,
                labels: snapshot.labels.clone(),
                props: HashMap::new(),
            });
        }
        let mut targets = HashMap::new();
        targets.insert(reference.id().to_owned(), node);
        for stub in &snapshot.stubs {
            let target = if let Some(existing) = mirror_node(&graph, reference.graph(), &stub.id) {
                existing
            } else if let Some(existing) = targets.get(&stub.id) {
                *existing
            } else {
                let id = next.0;
                next.0 += 1;
                ops.push(Operation::InsertNode {
                    id,
                    labels: stub.labels.clone(),
                    props: HashMap::new(),
                });
                ops.push(operation(&Metadata::Mirror {
                    node: id,
                    mirror: Mirror {
                        source: reference.graph().into(),
                        id: stub.id.clone(),
                        version: 0,
                        source_gone: false,
                        relationships: Vec::new(),
                        stub: true,
                    },
                }));
                id
            };
            targets.insert(stub.id.clone(), target);
        }
        if let Some(before) = graph.linked.mirrors.get(&node) {
            for id in &before.relationships {
                ops.push(Operation::DeleteRel { id: *id });
            }
        }
        let mut relationships = Vec::new();
        for rel in &snapshot.rels {
            let to = targets
                .get(&rel.to)
                .copied()
                .ok_or_else(|| failure("source relationship has no one-hop stub"))?;
            let id = next.1;
            next.1 += 1;
            ops.push(Operation::InsertRel {
                id,
                kind: rel.kind.clone(),
                from: node,
                to,
                props: HashMap::new(),
            });
            relationships.push(id);
        }
        ops.push(operation(&Metadata::Mirror {
            node,
            mirror: Mirror {
                source: reference.graph().into(),
                id: snapshot.id.clone(),
                version: snapshot.version,
                source_gone: snapshot.source_gone,
                relationships,
                stub: false,
            },
        }));
        self.commit_linked(&mut graph, ops)?;
        Ok(node)
    }
    fn commit_linked(&self, graph: &mut Graph, mut ops: Vec<Operation>) -> crate::Result<()> {
        if ops.is_empty() {
            return Ok(());
        }
        // Replica facts are not re-published as this graph's own source
        // changes, but every committed local write advances its graph cursor.
        let version = graph
            .linked
            .graph_version
            .checked_add(1)
            .ok_or_else(|| failure("graph version exhausted"))?;
        ops.push(operation(&Metadata::Commit {
            record: Record::scoped(version, now_secs(), Vec::new()),
        }));
        self.wal.append_statement(ops.clone())?;
        for op in &ops {
            crate::apply_op_to_memory(graph, op, std::path::Path::new(""))?;
        }
        Ok(())
    }
    pub fn mirrors(&self, source: &str) -> crate::Result<Vec<(NodeId, Mirror)>> {
        Ok(self
            .lock_graph()?
            .linked
            .mirrors
            .iter()
            .filter(|(_, m)| m.source == source)
            .map(|(id, m)| (*id, m.clone()))
            .collect())
    }
    /// Apply only newer versions to already linked mirrors. Unknown nodes are
    /// never installed by a push. The receiver authenticates the source.
    pub fn apply_diff(&self, diff: &Diff) -> crate::Result<()> {
        let mut seen = std::collections::HashSet::new();
        if diff.changes.iter().any(|c| !seen.insert(&c.id)) {
            return Err(failure("a diff must contain at most one change per node"));
        }
        let mut graph = self.lock_graph()?;
        let mut ops = Vec::new();
        let mut next = graph.next_ids();
        let targets: HashMap<String, NodeId> = graph
            .linked
            .mirrors
            .iter()
            .filter(|(_, m)| m.source == diff.source)
            .map(|(id, m)| (m.id.clone(), *id))
            .collect();
        for change in &diff.changes {
            let Some(node) = targets.get(&change.id).copied() else {
                continue;
            };
            let Some(before) = graph.linked.mirrors.get(&node) else {
                continue;
            };
            if change.version <= before.version || before.stub {
                continue;
            }
            if change.version > before.version.saturating_add(1) {
                return Err(failure("mirror version gap: refetch the source snapshot before applying partial fields"));
            }
            let mut mirror = before.clone();
            mirror.version = change.version;
            match &change.change {
                Change::Delete => {
                    mirror.source_gone = true;
                }
                Change::Upsert { fields, rels } => {
                    mirror.source_gone = false;
                    ops.push(Operation::UpdateNode {
                        id: node,
                        props: fields
                            .iter()
                            .map(|(k, v)| {
                                let shape = graph
                                    .get_node(node)
                                    .and_then(|node| node.prop(k))
                                    .and_then(value_shape);
                                Ok((k.clone(), typed_json(v, shape.as_ref())?))
                            })
                            .collect::<crate::Result<_>>()?,
                    });
                    for rel in &rels.remove {
                        mirror.relationships.retain(|id| {
                            let remove = graph.get_relationship(*id).is_some_and(|r| {
                                r.kind == rel.kind && targets.get(&rel.to) == Some(&r.to)
                            });
                            if remove {
                                ops.push(Operation::DeleteRel { id: *id });
                            }
                            !remove
                        });
                    }
                    for rel in &rels.add {
                        let target = *targets.get(&rel.to).ok_or_else(|| {
                            failure("new relationship target requires a one-hop source snapshot")
                        })?;
                        if mirror.relationships.iter().any(|id| {
                            graph
                                .get_relationship(*id)
                                .is_some_and(|r| r.kind == rel.kind && r.to == target)
                        }) {
                            continue;
                        }
                        let id = next.1;
                        next.1 += 1;
                        ops.push(Operation::InsertRel {
                            id,
                            kind: rel.kind.clone(),
                            from: node,
                            to: target,
                            props: HashMap::new(),
                        });
                        mirror.relationships.push(id);
                    }
                }
            }
            ops.push(operation(&Metadata::Mirror { node, mirror }));
        }
        self.commit_linked(&mut graph, ops)
    }
}

/// Merge a window in commit order, keeping only final field values and edge
/// membership. Used for every subscriber and by the proof harness.
pub fn coalesce(source: &str, records: &[Record]) -> Diff {
    let mut nodes = BTreeMap::<String, NodeChange>::new();
    for record in records {
        for change in &record.changes {
            if let Some(previous) = nodes.get_mut(&change.id) {
                if let (
                    Change::Upsert {
                        fields: a,
                        rels: ar,
                    },
                    Change::Upsert {
                        fields: b,
                        rels: br,
                    },
                ) = (&mut previous.change, &change.change)
                {
                    a.extend(b.clone());
                    for (add, rels) in [(true, &br.add), (false, &br.remove)] {
                        for rel in rels {
                            let (insert, remove) = if add {
                                (&mut ar.add, &mut ar.remove)
                            } else {
                                (&mut ar.remove, &mut ar.add)
                            };
                            remove.retain(|r| r != rel);
                            if !insert.contains(rel) {
                                insert.push(rel.clone());
                            }
                        }
                    }
                    previous.version = change.version;
                    continue;
                }
            }
            nodes.insert(change.id.clone(), change.clone());
        }
    }
    Diff {
        source: source.into(),
        graph_version: records.last().map_or(0, |r| r.graph_version),
        changes: nodes.into_values().collect(),
    }
}

/// Reconcile a source reload by external key, regardless of numeric slots in
/// the incoming file. Existing versions, tombstones, leases and retained WAL
/// history belong to this graph, not to the file being loaded.
pub(crate) fn reconcile(current: &Graph, incoming: &Graph) -> crate::Result<Store> {
    // Ordinary import/export retains its established byte-for-byte contract.
    // Once publicly linked, this graph owns its version history across reloads.
    if !current.linked.enabled {
        return Ok(incoming.linked.clone());
    }
    if !current.linked.mirrors.is_empty() {
        return Err(failure("a graph containing mirrors cannot be replaced; update local facts or reopen its checkpoint"));
    }
    let mut store = current.linked.clone();
    store.external.clear();
    let mut nodes = BTreeMap::new();
    for node in incoming.nodes() {
        if incoming.linked.mirrors.contains_key(&node.id) {
            continue;
        }
        let external = incoming
            .linked
            .external
            .get(&node.id)
            .cloned()
            .or_else(|| {
                node.prop("id")
                    .and_then(Value::as_string)
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| format!("n{}-{}", store.graph_version.saturating_add(1), node.id));
        Reference::new("graph", &external).map_err(|e| failure(e.to_string()))?;
        if nodes.insert(external.clone(), node.id).is_some() {
            return Err(failure("duplicate external id in source reload"));
        }
        store.external.insert(node.id, external.clone());
        store.identities.insert(external, node.id);
    }
    let mut changes = Vec::new();
    for (external, id) in &nodes {
        let node = incoming.get_node(*id).expect("enumerated node");
        let old = current
            .linked
            .identities
            .get(external)
            .filter(|id| current.linked.external.get(id) == Some(external))
            .and_then(|id| current.get_node(*id));
        if old.is_some_and(|old| {
            old.labels().collect::<Vec<_>>() != node.labels().collect::<Vec<_>>()
        }) {
            return Err(failure(
                "source reload cannot change the type of a stable external id",
            ));
        }
        let mut fields = BTreeMap::new();
        for (key, value) in node.props() {
            if old.and_then(|old| old.prop(key)) != Some(value) {
                fields.insert(key.into(), crate::v2::value_to_json(value));
            }
        }
        if let Some(old) = old {
            for (key, _) in old.props() {
                if node.prop(key).is_none() {
                    fields.insert(key.into(), serde_json::Value::Null);
                }
            }
        }
        let edges = |graph: &Graph, id: NodeId, external: &HashMap<NodeId, String>| {
            graph
                .node_relationship_ids(id)
                .into_iter()
                .filter_map(|id| graph.get_relationship(id))
                .filter(|r| r.from == id)
                .filter_map(|r| {
                    external.get(&r.to).map(|to| Relationship {
                        kind: r.kind.into(),
                        to: to.clone(),
                    })
                })
                .collect::<Vec<_>>()
        };
        let after = edges(incoming, *id, &store.external);
        let before = old
            .map(|old| edges(current, old.id, &current.linked.external))
            .unwrap_or_default();
        let rels = RelationshipChanges {
            add: after
                .iter()
                .filter(|r| !before.contains(r))
                .cloned()
                .collect(),
            remove: before
                .iter()
                .filter(|r| !after.contains(r))
                .cloned()
                .collect(),
        };
        if old.is_none() || !fields.is_empty() || !rels.add.is_empty() || !rels.remove.is_empty() {
            changes.push(NodeChange {
                id: external.clone(),
                version: store
                    .versions
                    .get(external)
                    .copied()
                    .unwrap_or(0)
                    .checked_add(1)
                    .ok_or_else(|| failure("node version exhausted"))?,
                kind: NodeKind::Node,
                change: Change::Upsert { fields, rels },
            });
        }
    }
    for (external, id) in &current.linked.identities {
        if !nodes.contains_key(external)
            && current.get_node(*id).is_some()
            && current.linked.external.get(id) == Some(external)
        {
            changes.push(NodeChange {
                id: external.clone(),
                version: store
                    .versions
                    .get(external)
                    .copied()
                    .unwrap_or(0)
                    .checked_add(1)
                    .ok_or_else(|| failure("node version exhausted"))?,
                kind: NodeKind::Node,
                change: Change::Delete,
            });
        }
    }
    if !changes.is_empty() {
        store.graph_version = store
            .graph_version
            .checked_add(1)
            .ok_or_else(|| failure("graph version exhausted"))?;
        for change in &changes {
            store.versions.insert(change.id.clone(), change.version);
        }
        store.history.push(Record {
            graph_version: store.graph_version,
            committed_at: now_secs(),
            changes,
            identities: nodes.into_iter().collect(),
        });
    }
    Ok(store)
}
impl Zega {
    /// Register stable ids for data written by engines predating APS 39.
    pub fn prepare_linked(&self) -> crate::Result<()> {
        let mut graph = self.lock_graph()?;
        if graph.linked.enabled {
            return Ok(());
        }
        let missing = graph
            .nodes()
            .filter(|node| {
                !graph.linked.external.contains_key(&node.id)
                    && !graph.linked.mirrors.contains_key(&node.id)
            })
            .map(|node| {
                let node = node.to_node();
                Operation::InsertNode {
                    id: node.id,
                    labels: node.labels,
                    props: node.props,
                }
            })
            .collect::<Vec<_>>();
        let mut metadata = Vec::new();
        if let Some(record) = record(&graph, &missing, &[])? {
            metadata.push(Metadata::Commit { record });
        }
        metadata.push(Metadata::Enable);
        self.wal
            .append_statement(metadata.iter().map(operation).collect())?;
        for item in metadata {
            apply_metadata(&mut graph, item);
        }
        Ok(())
    }
}

#[cfg(test)]
mod storage_tests {
    use super::*;
    #[test]
    fn checkpoint_retains_thirty_days_and_expires_leases_without_reusing_versions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_str().unwrap();
        let db = Zega::open(path).snapshot_every(0).build().unwrap();
        let schema = "type Thing { id: String n: Int }";
        db.run_lang(schema, r#"mutation { Thing(id: "Q1" && n: 1) { id } }"#)
            .unwrap();
        db.run_lang(schema, r#"mutation { Thing(id: "Q1") set n: 2 { n } }"#)
            .unwrap();
        db.subscribe(&SubscribeRequest {
            subscriber: "expired".into(),
            endpoint: None,
            ids: vec!["Q1".into()],
            lease_secs: 1,
        })
        .unwrap();
        let version = db.graph_version().unwrap();
        {
            let mut graph = db.lock_graph().unwrap();
            graph.linked.history[0].committed_at = now_secs() - MAILBOX_TTL.as_secs() - 1;
            graph.linked.history[1].committed_at = now_secs() - MAILBOX_TTL.as_secs() + 86400;
            std::sync::Arc::make_mut(graph.linked.leases.get_mut("Q1").unwrap())
                .get_mut("expired")
                .unwrap()
                .expires_at = now_secs() - 1;
        }
        db.checkpoint().unwrap();
        drop(db);
        let db = Zega::open(path).snapshot_every(0).build().unwrap();
        assert_eq!(db.graph_version().unwrap(), version);
        assert_eq!(db.changes_since(0).unwrap().len(), 1);
        assert!(db.subscribers(&["Q1".into()]).unwrap()["Q1"].is_empty());
        assert_eq!(
            db.sync_check(&CheckRequest {
                items: vec![("Q1".into(), 1)]
            })
            .unwrap()
            .stale,
            vec!["Q1"]
        );
    }
}
