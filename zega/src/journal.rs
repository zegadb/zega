//! One statement's writes, held back from readers until the WAL accepts them.
//!
//! Every write path (the ZQL mutations, loads and API calls in `v2.rs`) goes
//! through [`atomically`]:
//!
//! 1. The statement runs under the graph lock and makes its changes through a
//!    [`Journal`], which records the WAL operation for each change and what it
//!    replaced. Later clauses of the same statement read the earlier ones.
//! 2. When the statement has finished without error, the journal appends ALL
//!    of its operations as ONE WAL entry (a single CRC), so a statement is
//!    either wholly on disk or not there at all, also across a crash.
//! 3. If the statement fails, or the WAL refuses the entry, the journal puts
//!    back every node, relationship and id counter it touched, through the
//!    same graph methods that keep the label, property, unique and declared
//!    indexes current. The lock is held from step 1 to here, so no reader
//!    ever sees a change the WAL did not accept.
//!
//! So every statement is all-or-nothing, in memory and on disk.

use std::collections::HashMap;

use crate::graph::{Graph, Node, NodeId, RelId, Relationship};
use crate::value::Value;
use crate::wal::{Operation, Wal, WalError};

/// What one change replaced, so a refused statement can be taken back.
enum Undo {
    RelationshipHistory(RelId, Option<crate::history::RelHistory>),
    Lifetime(NodeId, Option<i64>, Option<i64>),
    History(NodeId, Vec<(String, crate::history::History)>),
    /// The statement created this node: remove it.
    Created(NodeId),
    /// The statement updated or deleted this node: put this copy back.
    Replaced(Node),
    /// The statement created this relationship: remove it.
    CreatedRel(RelId),
    /// The statement deleted this relationship: put this copy back.
    DeletedRel(Relationship),
}

pub(crate) struct Journal {
    ops: Vec<Operation>,
    undo: Vec<Undo>,
    next_ids: (NodeId, RelId),
    temporal: Vec<(String, String)>,
    temporal_rels: Vec<String>,
    lifetimes: Vec<(String, Option<String>, Option<String>)>,
    at: i64,
    history_error: Option<String>,
}

/// Run one statement's writes against `graph` and make them durable as a
/// unit. Returns the statement's result only once the WAL has accepted every
/// write; on any error, `graph` is exactly what it was before the call.
pub(crate) fn atomically<T, E: From<WalError>>(
    graph: &mut Graph,
    wal: &Wal,
    statement: impl FnOnce(&mut Graph, &mut Journal) -> Result<T, E>,
) -> Result<T, E> {
    let mut journal = Journal {
        ops: Vec::new(),
        undo: Vec::new(),
        next_ids: graph.next_ids(),
        temporal: Vec::new(),
        temporal_rels: Vec::new(),
        lifetimes: Vec::new(),
        at: crate::history::now(),
        history_error: None,
    };
    let result = statement(graph, &mut journal);
    let result = match journal.history_error.take() {
        Some(reason) => Err(WalError::Corruption { offset: 0, reason }.into()),
        None => result,
    };
    match result {
        Ok(value) => {
            let Journal {
                ops,
                undo,
                next_ids,
                ..
            } = journal;
            if let Err(error) = wal.append_statement(ops) {
                roll_back(graph, undo, next_ids);
                return Err(error.into());
            }
            Ok(value)
        }
        Err(error) => {
            roll_back(graph, journal.undo, journal.next_ids);
            Err(error)
        }
    }
}

fn roll_back(graph: &mut Graph, undo: Vec<Undo>, next_ids: (NodeId, RelId)) {
    for change in undo.into_iter().rev() {
        match change {
            Undo::RelationshipHistory(id, before) => {
                let h = graph.history.get_mut().expect("decoded history");
                if let Some(before) = before {
                    h.relationships.insert(id, before);
                } else {
                    h.relationships.remove(&id);
                }
            }
            Undo::Lifetime(id, appears, ends) => {
                let h = graph.history.get_mut().expect("decoded history");
                for (dates, value) in [(&mut h.appears, appears), (&mut h.ends, ends)] {
                    if let Some(value) = value {
                        dates.insert(id, value);
                    } else {
                        dates.remove(&id);
                    }
                }
            }
            Undo::History(id, before) => {
                let histories = graph
                    .history
                    .get_mut()
                    .expect("history was decoded before mutation");
                histories.retain(|(node, _), _| *node != id);
                for (field, h) in before {
                    histories.insert((id, field), h);
                }
            }
            Undo::Created(id) => graph.delete_node(id),
            Undo::Replaced(node) => graph.restore_node(node.id, node.labels, node.props),
            Undo::CreatedRel(id) => graph.delete_relationship(id),
            Undo::DeletedRel(rel) => {
                graph.restore_relationship(rel.id, rel.kind, rel.from, rel.to, rel.props)
            }
        }
    }
    graph.reset_next_ids(next_ids);
}

impl Journal {
    pub(crate) fn configure_time(&mut self, schema: &crate::lang::Schema, at: Option<i64>) {
        self.at = at.unwrap_or_else(crate::history::now);
        self.temporal_rels = schema
            .types
            .iter()
            .flat_map(|t| t.fields.iter())
            .filter_map(|f| match f {
                crate::lang::Field::Edge {
                    rel,
                    temporal: true,
                    ..
                } => Some(rel.clone()),
                _ => None,
            })
            .collect();
        self.lifetimes = schema
            .types
            .iter()
            .filter(|t| t.appears.is_some() || t.ends.is_some())
            .map(|t| (t.name.clone(), t.appears.clone(), t.ends.clone()))
            .collect();
        self.temporal = schema
            .types
            .iter()
            .flat_map(|t| {
                t.fields.iter().filter_map(|f| match f {
                    crate::lang::Field::Prop { name, ty, .. }
                        if crate::history::is_temporal(ty) =>
                    {
                        Some((t.name.clone(), name.clone()))
                    }
                    _ => None,
                })
            })
            .collect();
    }
    fn fields(&self, labels: &[String], props: &HashMap<String, Value>) -> Vec<String> {
        self.temporal
            .iter()
            .filter(|(ty, f)| labels.contains(ty) && props.contains_key(f))
            .map(|(_, f)| f.clone())
            .collect()
    }
    fn save_history(&mut self, graph: &mut Graph, id: NodeId) -> bool {
        match graph.history.get() {
            Ok(h) => {
                self.undo.push(Undo::History(
                    id,
                    h.iter()
                        .filter(|((n, _), _)| *n == id)
                        .map(|((_, f), h)| (f.clone(), h.clone()))
                        .collect(),
                ));
                true
            }
            Err(e) => {
                self.history_error = Some(e);
                false
            }
        }
    }

    pub(crate) fn create_node(
        &mut self,
        graph: &mut Graph,
        labels: Vec<String>,
        props: HashMap<String, Value>,
    ) -> NodeId {
        let id = graph.create_node(labels.clone(), props.clone());
        self.undo.push(Undo::Created(id));
        let fields = self.fields(&labels, &props);
        if fields.is_empty() {
            self.ops.push(Operation::InsertNode { id, labels, props });
        } else if self.save_history(graph, id) {
            graph
                .record_history(id, self.at, &props, &fields)
                .expect("decoded history");
            self.ops.push(Operation::InsertNodeAt {
                id,
                labels,
                props,
                at: self.at,
                fields,
            });
        }
        self.record_lifetime(graph, id);
        id
    }

    fn record_lifetime(&mut self, graph: &mut Graph, id: NodeId) {
        let Some(node) = graph.get_node(id) else {
            return;
        };
        let Some((_, appears, ends)) = self.lifetimes.iter().find(|(ty, _, _)| node.has_label(ty))
        else {
            return;
        };
        let read = |field: &Option<String>| -> Option<i64> {
            field
                .as_ref()
                .and_then(|f| node.prop(f))
                .and_then(|v| match v {
                    Value::String(s) => crate::history::date(s).ok(),
                    _ => None,
                })
        };
        let (appears, ends) = (read(appears), read(ends));
        if appears.zip(ends).is_some_and(|(a, e)| a > e) {
            self.history_error = Some("lifetime ends before it appears".into());
            return;
        }
        match graph.history.get_mut() {
            Ok(h) => {
                self.undo.push(Undo::Lifetime(
                    id,
                    h.appears.get(&id).copied(),
                    h.ends.get(&id).copied(),
                ));
                for (dates, value) in [(&mut h.appears, appears), (&mut h.ends, ends)] {
                    if let Some(value) = value {
                        dates.insert(id, value);
                    } else {
                        dates.remove(&id);
                    }
                }
                self.ops.push(Operation::SetLifetime { id, appears, ends });
            }
            Err(e) => self.history_error = Some(e),
        }
    }

    /// Merge `props` into a stored node. A missing node is left alone and
    /// nothing is logged, which is what replaying the update would do.
    pub(crate) fn update_node(
        &mut self,
        graph: &mut Graph,
        id: NodeId,
        props: HashMap<String, Value>,
    ) {
        let Some(before) = graph.get_node(id).map(|node| node.to_node()) else {
            return;
        };
        let fields = self.fields(&before.labels, &props);
        if fields.is_empty() {
            graph.update_node(id, props.clone());
            self.undo.push(Undo::Replaced(before));
            self.ops.push(Operation::UpdateNode { id, props });
        } else if self.save_history(graph, id) {
            let mut current = props.clone();
            graph
                .record_history(id, self.at, &props, &fields)
                .expect("decoded history");
            for field in &fields {
                if let Some(h) = graph
                    .history
                    .get()
                    .expect("decoded history")
                    .get(&(id, field.clone()))
                {
                    current.insert(
                        field.clone(),
                        h.changes.last().expect("nonempty history").1.clone(),
                    );
                }
            }
            graph.update_node(id, current);
            self.undo.push(Undo::Replaced(before));
            self.ops.push(Operation::UpdateNodeAt {
                id,
                props,
                at: self.at,
                fields,
            });
        }
        self.record_lifetime(graph, id);
    }

    /// Delete a node and every relationship still attached to it.
    pub(crate) fn delete_node(&mut self, graph: &mut Graph, id: NodeId) {
        let Some(before) = graph.get_node(id).map(|node| node.to_node()) else {
            return;
        };
        for rel in graph.node_relationship_ids(id) {
            self.delete_relationship(graph, rel);
        }
        if graph.history.has_data() {
            if !self.save_history(graph, id) {
                return;
            }
            let h = graph.history.get_mut().expect("decoded history");
            h.retain(|(node, _), _| *node != id);
            self.undo.push(Undo::Lifetime(
                id,
                h.appears.remove(&id),
                h.ends.remove(&id),
            ));
            let incident: Vec<_> = h
                .relationships
                .iter()
                .filter(|(_, r)| r.rel.from == id || r.rel.to == id)
                .map(|(id, _)| *id)
                .collect();
            for rel in incident {
                self.undo
                    .push(Undo::RelationshipHistory(rel, h.relationships.remove(&rel)));
            }
        }
        graph.delete_node(id);
        self.undo.push(Undo::Replaced(before));
        self.ops.push(Operation::DeleteNode { id });
    }

    pub(crate) fn create_relationship(
        &mut self,
        graph: &mut Graph,
        kind: String,
        from: NodeId,
        to: NodeId,
        props: HashMap<String, Value>,
    ) -> RelId {
        let id = graph.create_relationship(kind.clone(), from, to, props.clone());
        self.undo.push(Undo::CreatedRel(id));
        if self.temporal_rels.contains(&kind) {
            let rel = Relationship {
                id,
                kind,
                from,
                to,
                props,
            };
            match graph.history.get_mut() {
                Ok(h) => {
                    self.undo.push(Undo::RelationshipHistory(
                        id,
                        h.relationships.get(&id).cloned(),
                    ));
                    h.relationships.insert(
                        id,
                        crate::history::RelHistory {
                            rel: rel.clone(),
                            from: self.at,
                            to: None,
                        },
                    );
                    self.ops.push(Operation::InsertRelAt { rel, at: self.at });
                }
                Err(e) => self.history_error = Some(e),
            }
        } else {
            self.ops.push(Operation::InsertRel {
                id,
                kind,
                from,
                to,
                props,
            });
        }
        id
    }

    pub(crate) fn relationship_time(&self, kind: &str) -> Option<i64> {
        self.temporal_rels
            .iter()
            .any(|rel| rel == kind)
            .then_some(self.at)
    }

    pub(crate) fn delete_relationship(&mut self, graph: &mut Graph, id: RelId) {
        let was_live = graph.get_relationship(id).is_some();
        let before = graph.get_relationship(id).map(|rel| rel.to_relationship());
        let before = if before.is_none() && graph.history.has_data() {
            match graph.history.get() {
                Ok(h) => h.relationships.get(&id).map(|h| h.rel.clone()),
                Err(e) => {
                    self.history_error = Some(e);
                    return;
                }
            }
        } else {
            before
        };
        let Some(before) = before else {
            return;
        };
        let temporal = self.temporal_rels.contains(&before.kind)
            || (graph.history.has_data()
                && match graph.history.get() {
                    Ok(h) => h.relationships.contains_key(&id),
                    Err(e) => {
                        self.history_error = Some(e);
                        return;
                    }
                });
        if temporal {
            match graph.history.get_mut() {
                Ok(h) => {
                    let previous = h.relationships.get(&id).cloned();
                    if previous.as_ref().is_some_and(|r| self.at < r.from) {
                        self.history_error = Some("relationship ends before it starts".into());
                        return;
                    }
                    self.undo.push(Undo::RelationshipHistory(id, previous));
                    let entry =
                        h.relationships
                            .entry(id)
                            .or_insert_with(|| crate::history::RelHistory {
                                rel: before.clone(),
                                from: i64::MIN,
                                to: None,
                            });
                    entry.to = Some(self.at);
                    self.ops.push(Operation::EndRelAt {
                        rel: before.clone(),
                        at: self.at,
                    });
                }
                Err(e) => {
                    self.history_error = Some(e);
                    return;
                }
            }
        } else {
            self.ops.push(Operation::DeleteRel { id });
        }
        graph.delete_relationship(id);
        if was_live {
            self.undo.push(Undo::DeletedRel(before));
        }
    }
}
