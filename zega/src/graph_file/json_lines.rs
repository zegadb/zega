//! JSON lines: the bulk-load input (zegadb/zega#127, #128). A whole graph,
//! one node or relationship per line, read in one streaming pass into a new
//! [`Graph`]:
//!
//! ```text
//! {"type":"node","id":42,"labels":["Item"],"props":{"qid":"Q42","name":"Douglas Adams"}}
//! {"type":"rel","kind":"P31","from":42,"to":5,"props":{"rank":"normal"}}
//! ```
//!
//! It is what an intake produces (a filtered Wikidata dump, a crawl): a
//! format any language writes one record at a time, with no dictionary,
//! counts or checksums to compute ahead. docs/data-loading.md#bulk-loading
//! specifies it. [`crate::Zega::import_json_lines`] loads it with the same
//! guarantees as a `.graph` import: the graph is installed only once every
//! line has been read and checked, and a disk database stores it as a
//! `.graph` file committed by one WAL entry.
//!
//! Memory is the graph being built plus one line: no line, section or file
//! is held beyond the one being read, so a load's peak is close to the
//! graph's resting size (#128), whatever the input's size (#127).

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read};

use serde_json::Value as Json;

use super::{duplicate_label, schema_declarations, Carried, Error, ImportSummary, IndexDeclaration, BUFFER, FORMAT_VERSION};
use crate::graph::Graph;
use crate::value::Value;

/// The longest line read, in bytes: well beyond any one node or
/// relationship an intake writes, and it bounds what a malformed input
/// (a file with no newlines) can make the reader hold.
pub const MAX_LINE_BYTES: usize = 16 << 20;

/// What a JSON-lines load carries besides its lines, as a `.graph` file
/// does (docs/graph-format.md, `SCHM` and `MNFT`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct JsonLinesOptions {
    /// ZQL schema text. It types each node's properties the way `mutation
    /// json` does (by the node's first label: `T[]` lists, vectors,
    /// references), its `unique` fields are checked across the whole load,
    /// its indexes are built, and the graph carries it as an imported
    /// `.graph` file's schema.
    pub schema: Option<String>,
    /// Manifest metadata (`title`, `source`, `licence`, …).
    pub meta: BTreeMap<String, String>,
}

/// One line, before it is checked: the fields a node or a relationship may
/// have. Anything else is refused.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Line {
    #[serde(rename = "type")]
    what: What,
    id: Option<u64>,
    labels: Option<Vec<String>>,
    kind: Option<String>,
    from: Option<u64>,
    to: Option<u64>,
    #[serde(default)]
    props: serde_json::Map<String, Json>,
}

#[derive(serde::Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum What {
    Node,
    Rel,
}

/// Read a JSON-lines graph from `input` into a new graph. The graph is
/// returned only when every line has been read and checked.
/// `content_sha256` in the summary is left empty: it is the digest of the
/// `.graph` file the caller writes.
pub(crate) fn read<R: Read>(input: R, options: &JsonLinesOptions) -> Result<(Graph, ImportSummary), Error> {
    let invalid = |line: u64, reason: String| Error::JsonLines { line, reason };
    let (schema, uniques, indexes) = match &options.schema {
        Some(source) => {
            let (schema, (uniques, indexes)) =
                schema_declarations(source).map_err(|error| invalid(0, format!("the schema does not parse: {error}")))?;
            (Some(schema), uniques, indexes)
        }
        None => (None, Vec::new(), Vec::new()),
    };
    let mut graph = Graph::new();
    // Filled as nodes arrive, so each unique value is checked against every
    // node before it in one lookup.
    graph.sync_uniques(&uniques);

    let mut input = BufReader::with_capacity(BUFFER, input);
    let mut buf = Vec::new();
    let (mut number, mut nodes, mut relationships) = (0u64, 0u64, 0u64);
    while next_line(&mut input, &mut buf).map_err(|reason| invalid(number + 1, reason))? {
        number += 1;
        let text = buf.strip_suffix(b"\r").unwrap_or(&buf);
        if text.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let line: Line = serde_json::from_slice(text).map_err(|error| invalid(number, error.to_string()))?;
        let at = |reason: String| invalid(number, reason);
        match line.what {
            What::Node => {
                if line.kind.is_some() || line.from.is_some() || line.to.is_some() {
                    return Err(at("a node has no kind, from or to".into()));
                }
                let labels = line.labels.unwrap_or_default();
                if let Some(label) = duplicate_label(&labels) {
                    return Err(at(format!("the node has the label {label:?} twice")));
                }
                let id = line.id.unwrap_or_else(|| graph.next_ids().0);
                if graph.get_node(id).is_some() {
                    return Err(at(format!("node {id} is already defined by an earlier line")));
                }
                // ZQL's type for a node is its first label.
                let type_name = labels.first().map(String::as_str);
                let props = props(line.props, |field, value| match (&schema, type_name) {
                    (Some(schema), Some(type_name)) => {
                        crate::v2::json_to_field(schema, type_name, NO_SPAN, field, value)
                    }
                    _ => crate::v2::json_to_value(value),
                })
                .map_err(at)?;
                for (ty, field) in &uniques {
                    let Some(value) = props.get(field).filter(|value| !matches!(value, Value::Null)) else {
                        continue;
                    };
                    if !labels.iter().any(|label| label == ty) {
                        continue;
                    }
                    if let Some(other) = graph.unique_matches(ty, field, value).first() {
                        return Err(at(format!(
                            "node {id}'s {ty}.{field} is {}, which node {other} already has: the schema makes {ty}.{field} unique",
                            crate::v2::value_to_json(value)
                        )));
                    }
                }
                graph.restore_node(id, labels, props);
                nodes += 1;
            }
            What::Rel => {
                if line.labels.is_some() {
                    return Err(at("a relationship has a kind, not labels".into()));
                }
                let kind = line.kind.ok_or_else(|| at("a relationship needs a kind".into()))?;
                let from = line.from.ok_or_else(|| at("a relationship needs from (a node id)".into()))?;
                let to = line.to.ok_or_else(|| at("a relationship needs to (a node id)".into()))?;
                for (end, node) in [("from", from), ("to", to)] {
                    if graph.get_node(node).is_none() {
                        return Err(at(format!(
                            "the relationship's {end} is node {node}, which no earlier line defines \
                             (nodes come before the relationships that use them)"
                        )));
                    }
                }
                let id = line.id.unwrap_or_else(|| graph.next_ids().1);
                if graph.get_relationship(id).is_some() {
                    return Err(at(format!("relationship {id} is already defined by an earlier line")));
                }
                let props = props(line.props, |_, value| crate::v2::json_to_value(value)).map_err(at)?;
                graph.restore_relationship(id, kind, from, to, props);
                relationships += 1;
            }
        }
    }

    graph.sync_indexes(&indexes);
    let carried = Carried {
        schema: options.schema.clone(),
        uniques: uniques.clone(),
        indexes: indexes.clone(),
        meta: options.meta.clone(),
    };
    graph.set_carried(carried);
    let summary = ImportSummary {
        format_version: FORMAT_VERSION,
        created_by: crate::CREATED_BY.to_string(),
        nodes,
        relationships,
        schema: options.schema.clone(),
        uniques,
        indexes: indexes
            .iter()
            .map(|spec| IndexDeclaration {
                kind: spec.kind.as_str(),
                type_name: spec.type_name.clone(),
                field: spec.field.clone(),
            })
            .collect(),
        meta: options.meta.clone(),
        content_sha256: String::new(),
    };
    Ok((graph, summary))
}

/// Read the next line into `buf`, without its `\n`. `Ok(false)` at the end
/// of the input. A line longer than [`MAX_LINE_BYTES`] is refused before
/// more of it is read.
fn next_line(input: &mut impl BufRead, buf: &mut Vec<u8>) -> Result<bool, String> {
    buf.clear();
    let mut read_any = false;
    loop {
        let available = input.fill_buf().map_err(|error| error.to_string())?;
        if available.is_empty() {
            return Ok(read_any);
        }
        read_any = true;
        let (chunk, done) = match available.iter().position(|&byte| byte == b'\n') {
            Some(at) => (&available[..at], at + 1),
            None => (available, available.len()),
        };
        if buf.len() + chunk.len() > MAX_LINE_BYTES {
            return Err(format!("the line is longer than {} MiB", MAX_LINE_BYTES >> 20));
        }
        buf.extend_from_slice(chunk);
        let newline = done > chunk.len();
        input.consume(done);
        if newline {
            return Ok(true);
        }
    }
}

/// Convert a line's properties, each by `convert`.
fn props(
    props: serde_json::Map<String, Json>,
    convert: impl Fn(&str, &Json) -> Result<Value, crate::lang::Error>,
) -> Result<std::collections::HashMap<String, Value>, String> {
    props
        .into_iter()
        .map(|(field, value)| {
            let converted = convert(&field, &value).map_err(|error| format!("property {field:?}: {}", error.message))?;
            Ok((field, converted))
        })
        .collect()
}

/// Errors from the shared conversion carry a position in the ZQL being run;
/// a line has none (its number goes in the error instead).
const NO_SPAN: crate::lang::Span = crate::lang::Span { line: 0, column: 0, end_line: 0, end_column: 0 };
