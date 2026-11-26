//! Node access for an embedder that moves whole tables at once. Reads see the
//! graph under its lock; writes go through a [`Transaction`], so they are
//! journaled, logged as the transaction's one WAL entry and taken back if it
//! fails, exactly as ZQL's are. ZQL text would round-trip every float through
//! a decimal string and the parser; these take [`Value`]s as they are stored.

use std::collections::HashMap;

use crate::graph::Graph;
use crate::journal::{atomically_into, Target};
use crate::{Result, Transaction, Value, Zega, ZegaError};

/// A read-only view of the graph's nodes.
pub struct Nodes<'g> {
    graph: &'g Graph,
}

impl<'g> Nodes<'g> {
    /// The ids of every node carrying `label`, ascending.
    pub fn labelled(&self, label: &str) -> Vec<u64> {
        let mut ids: Vec<u64> = self
            .graph
            .nodes_by_label(label)
            .map(|set| set.iter().copied().collect())
            .unwrap_or_default();
        ids.sort_unstable();
        ids
    }

    /// Whether node `id` exists.
    pub fn exists(&self, id: u64) -> bool {
        self.graph.get_node(id).is_some()
    }

    /// Node `id`'s property `key`, as stored.
    pub fn prop(&self, id: u64, key: &str) -> Option<&'g Value> {
        self.graph.get_node(id)?.prop(key)
    }
}

impl Zega {
    /// Run `f` over the graph's nodes, holding the graph lock while it runs.
    pub fn nodes<T>(&self, f: impl FnOnce(&Nodes<'_>) -> T) -> Result<T> {
        let graph = self.lock_graph()?;
        Ok(f(&Nodes { graph: &graph }))
    }
}

impl Transaction<'_> {
    /// The graph as this transaction sees it so far.
    pub fn nodes(&self) -> Nodes<'_> {
        Nodes { graph: self.graph() }
    }

    /// Create one node per `(labels, props)`; their ids, in order.
    pub fn create_nodes(&mut self, nodes: Vec<(Vec<String>, HashMap<String, Value>)>) -> Result<Vec<u64>> {
        let (graph, pending) = self.parts();
        let created = atomically_into(graph, Target::Pending(pending), |graph, journal| {
            Ok::<_, ZegaError>(
                nodes
                    .into_iter()
                    .map(|(labels, props)| journal.create_node(graph, labels, props))
                    .collect(),
            )
        });
        self.step(created)
    }

    /// Merge each `props` into node `id`'s properties. A missing node fails
    /// the transaction.
    pub fn update_nodes(&mut self, updates: Vec<(u64, HashMap<String, Value>)>) -> Result<()> {
        let (graph, pending) = self.parts();
        let updated = atomically_into(graph, Target::Pending(pending), |graph, journal| {
            for (id, props) in updates {
                if graph.get_node(id).is_none() {
                    return Err(ZegaError::Execution(format!("no node has id {id}")));
                }
                journal.update_node(graph, id, props);
            }
            Ok(())
        });
        self.step(updated)
    }
}
