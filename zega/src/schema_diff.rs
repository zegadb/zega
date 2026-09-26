//! Schema diff and breaking-change dry run (APS 23 step 1).
//!
//! Compares two schema texts and reports what would happen to the graph that
//! already stores data. The report is purely advisory: it counts real nodes,
//! properties and relationships, classifies each change as safe, a warning, or
//! blocking, and leaves the actual atomic apply and protected-graph flows to
//! APS 23 steps 2 and 3.
//!
//! Declared `unique`, `index` and `display` blocks are ignored for now; only
//! `type` definitions are compared.

use crate::graph::Graph;
use crate::lang::{Direction, EdgeField, Field, Schema, TypeDef};
use crate::value::Value;
use crate::vector::VectorSpec;
use serde::Serialize;
use std::collections::{HashMap, HashSet};

/// Whether a schema change can be applied without touching data.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Purely additive or relaxing; no data is at risk.
    Safe,
    /// Structural risk or a destructive change against an empty slice of data.
    Warn,
    /// Existing data would be lost or violated by the change.
    Blocks,
}

/// One classified change with a count taken from the real graph.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SchemaChange {
    pub severity: Severity,
    #[serde(flatten)]
    pub kind: ChangeKind,
    /// Number of nodes, values or relationships affected by this change.
    pub affected: u64,
    /// Human-readable summary, with thousands-separated numbers.
    pub message: String,
}

/// The structural change this entry describes.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ChangeKind {
    TypeAdded { name: String },
    TypeRemoved { name: String },
    TypeRenamed { from: String, to: String },
    FieldAdded { #[serde(rename = "type")] type_name: String, field: String, required: bool },
    FieldRemoved { #[serde(rename = "type")] type_name: String, field: String },
    FieldRenamed { #[serde(rename = "type")] type_name: String, from: String, to: String },
    FieldTypeChanged { #[serde(rename = "type")] type_name: String, field: String, from: String, to: String },
    FieldRequired { #[serde(rename = "type")] type_name: String, field: String },
    FieldRelaxed { #[serde(rename = "type")] type_name: String, field: String },
    RelationshipAdded { #[serde(rename = "type")] type_name: String, field: String, rel: String },
    RelationshipRemoved { #[serde(rename = "type")] type_name: String, field: String, rel: String },
    RelationshipChanged { #[serde(rename = "type")] type_name: String, field: String, from_rel: String, to_rel: String },
    EdgePropAdded { #[serde(rename = "type")] type_name: String, field: String, prop: String, required: bool },
    EdgePropRemoved { #[serde(rename = "type")] type_name: String, field: String, prop: String },
    EdgePropTypeChanged { #[serde(rename = "type")] type_name: String, field: String, prop: String, from: String, to: String },
    EdgePropRequired { #[serde(rename = "type")] type_name: String, field: String, prop: String },
    EdgePropRelaxed { #[serde(rename = "type")] type_name: String, field: String, prop: String },
}

/// The result of a dry-run schema comparison.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SchemaDiffReport {
    /// True when none of the changes are [`Severity::Blocks`].
    pub ok: bool,
    pub changes: Vec<SchemaChange>,
}

/// Compare `old` against `new` and report what would happen to `graph`.
pub fn diff_schemas(old: &Schema, new: &Schema, graph: &Graph) -> SchemaDiffReport {
    let mut changes = Vec::new();
    let old_types: HashMap<&str, &TypeDef> = old.types.iter().map(|t| (t.name.as_str(), t)).collect();
    let new_types: HashMap<&str, &TypeDef> = new.types.iter().map(|t| (t.name.as_str(), t)).collect();

    let rename = find_type_rename(old, new, &old_types, &new_types);
    let mut renamed_to = None;

    // Types in old-schema order: removals, renames, and per-type field diffs.
    for old_ty in &old.types {
        if let Some((from, to)) = rename {
            if old_ty.name == from {
                renamed_to = Some(to);
                let affected = count_nodes_with_label(graph, &old_ty.name);
                changes.push(SchemaChange {
                    severity: Severity::Warn,
                    kind: ChangeKind::TypeRenamed { from: from.to_string(), to: to.to_string() },
                    affected,
                    message: format!(
                        "renames type {} to {}: {} nodes preserved only if the apply step migrates them",
                        old_ty.name,
                        to,
                        fmt_num(affected)
                    ),
                });
                continue;
            }
        }

        if let Some(new_ty) = new_types.get(old_ty.name.as_str()) {
            diff_type_fields(graph, old_ty, new_ty, &mut changes);
        } else {
            let affected = count_nodes_with_label(graph, &old_ty.name);
            let severity = if affected > 0 { Severity::Blocks } else { Severity::Warn };
            changes.push(SchemaChange {
                severity,
                kind: ChangeKind::TypeRemoved { name: old_ty.name.clone() },
                affected,
                message: format!("removes type {}: {} nodes", old_ty.name, fmt_num(affected)),
            });
        }
    }

    // Added types in new-schema order, skipping the rename target.
    for new_ty in &new.types {
        if old_types.contains_key(new_ty.name.as_str()) {
            continue;
        }
        if renamed_to == Some(new_ty.name.as_str()) {
            continue;
        }
        let affected = count_nodes_with_label(graph, &new_ty.name);
        changes.push(SchemaChange {
            severity: Severity::Safe,
            kind: ChangeKind::TypeAdded { name: new_ty.name.clone() },
            affected,
            message: format!(
                "adds type {}: {} nodes already carry this label",
                new_ty.name,
                fmt_num(affected)
            ),
        });
    }

    let ok = changes.iter().all(|c| !matches!(c.severity, Severity::Blocks));
    SchemaDiffReport { ok, changes }
}

fn diff_type_fields(graph: &Graph, old_ty: &TypeDef, new_ty: &TypeDef, changes: &mut Vec<SchemaChange>) {
    let old_fields: HashMap<&str, &Field> = old_ty.fields.iter().map(|f| (field_name(f), f)).collect();
    let new_fields: HashMap<&str, &Field> = new_ty.fields.iter().map(|f| (field_name(f), f)).collect();

    let field_rename = find_field_rename(old_ty, new_ty, &old_fields, &new_fields);
    let mut renamed_field_to = None;

    for old_field in &old_ty.fields {
        if let Some((from, to)) = field_rename {
            if field_name(old_field) == from {
                renamed_field_to = Some(to);
                let affected = count_nodes_with_prop(graph, &old_ty.name, from);
                changes.push(SchemaChange {
                    severity: Severity::Warn,
                    kind: ChangeKind::FieldRenamed {
                        type_name: old_ty.name.clone(),
                        from: from.to_string(),
                        to: to.to_string(),
                    },
                    affected,
                    message: format!(
                        "renames {}.{} to {}: {} nodes have this property",
                        old_ty.name,
                        from,
                        to,
                        fmt_num(affected)
                    ),
                });
                continue;
            }
        }

        match old_field {
            Field::Prop { name, ty, optional, .. } => {
                if let Some(Field::Prop { ty: new_ty_str, optional: new_optional, .. }) = new_fields.get(name.as_str()) {
                    let old_base = base_type(ty);
                    let new_base = base_type(new_ty_str);
                    if old_base != new_base || !vector_compatible(ty, new_ty_str) {
                        let affected = count_non_converting(graph, &old_ty.name, name, new_ty_str);
                        let severity = if affected > 0 { Severity::Blocks } else { Severity::Warn };
                        changes.push(SchemaChange {
                            severity,
                            kind: ChangeKind::FieldTypeChanged {
                                type_name: old_ty.name.clone(),
                                field: name.clone(),
                                from: ty.clone(),
                                to: new_ty_str.clone(),
                            },
                            affected,
                            message: format!(
                                "changes {}.{} {} → {}: {} values don't convert",
                                old_ty.name,
                                name,
                                ty,
                                new_ty_str,
                                fmt_num(affected)
                            ),
                        });
                    } else if *optional && !*new_optional {
                        let affected = count_nodes_missing_prop(graph, &old_ty.name, name);
                        let severity = if affected > 0 { Severity::Blocks } else { Severity::Warn };
                        changes.push(SchemaChange {
                            severity,
                            kind: ChangeKind::FieldRequired {
                                type_name: old_ty.name.clone(),
                                field: name.clone(),
                            },
                            affected,
                            message: format!(
                                "makes {}.{} required: {} nodes have none",
                                old_ty.name,
                                name,
                                fmt_num(affected)
                            ),
                        });
                    } else if !*optional && *new_optional {
                        changes.push(SchemaChange {
                            severity: Severity::Safe,
                            kind: ChangeKind::FieldRelaxed {
                                type_name: old_ty.name.clone(),
                                field: name.clone(),
                            },
                            affected: 0,
                            message: format!("makes {}.{} optional", old_ty.name, name),
                        });
                    }
                } else {
                    let affected = count_nodes_with_prop(graph, &old_ty.name, name);
                    let severity = if affected > 0 { Severity::Blocks } else { Severity::Warn };
                    changes.push(SchemaChange {
                        severity,
                        kind: ChangeKind::FieldRemoved {
                            type_name: old_ty.name.clone(),
                            field: name.clone(),
                        },
                        affected,
                        message: format!(
                            "removes {}.{}",
                            old_ty.name, name
                        ),
                    });
                }
            }
            Field::Edge { field, rel, direction, targets, many, props, .. } => {
                if let Some(Field::Edge {
                    rel: new_rel,
                    direction: new_direction,
                    targets: new_targets,
                    many: new_many,
                    props: new_props,
                    ..
                }) = new_fields.get(field.as_str())
                {
                    if rel != new_rel
                        || direction != new_direction
                        || targets != new_targets
                        || many != new_many
                    {
                        let affected = count_relationships_changed(
                            graph,
                            &old_ty.name,
                            &EdgeDesc { rel, direction: *direction, targets },
                            &EdgeDesc { rel: new_rel, direction: *new_direction, targets: new_targets },
                        );
                        let severity = if affected > 0 { Severity::Blocks } else { Severity::Warn };
                        changes.push(SchemaChange {
                            severity,
                            kind: ChangeKind::RelationshipChanged {
                                type_name: old_ty.name.clone(),
                                field: field.clone(),
                                from_rel: rel.clone(),
                                to_rel: new_rel.clone(),
                            },
                            affected,
                            message: format!(
                                "changes {}.{} ({:?} {} -> {:?} {}): {} relationships",
                                old_ty.name,
                                field,
                                direction,
                                rel,
                                new_direction,
                                new_rel,
                                fmt_num(affected)
                            ),
                        });
                    } else {
                        diff_edge_props(
                            graph,
                            &old_ty.name,
                            field,
                            rel,
                            props,
                            new_props,
                            changes,
                        );
                    }
                } else {
                    let affected = count_rels_with_kind(graph, rel);
                    let severity = if affected > 0 { Severity::Blocks } else { Severity::Warn };
                    changes.push(SchemaChange {
                        severity,
                        kind: ChangeKind::RelationshipRemoved {
                            type_name: old_ty.name.clone(),
                            field: field.clone(),
                            rel: rel.clone(),
                        },
                        affected,
                        message: format!(
                            "removes {}.{} ({}): {} relationships",
                            old_ty.name,
                            field,
                            rel,
                            fmt_num(affected)
                        ),
                    });
                }
            }
        }
    }

    // Added fields in new-schema order, skipping a rename target.
    for new_field in &new_ty.fields {
        let name = field_name(new_field);
        if old_fields.contains_key(name) {
            continue;
        }
        if renamed_field_to == Some(name) {
            continue;
        }
        match new_field {
            Field::Prop { name, optional, ty: _, .. } => {
                let affected = count_nodes_missing_prop(graph, &old_ty.name, name);
                let severity = if *optional {
                    Severity::Safe
                } else if affected == 0 {
                    Severity::Warn
                } else {
                    Severity::Blocks
                };
                changes.push(SchemaChange {
                    severity,
                    kind: ChangeKind::FieldAdded {
                        type_name: old_ty.name.clone(),
                        field: name.clone(),
                        required: !*optional,
                    },
                    affected,
                    message: if *optional {
                        format!("adds optional {}.{}", old_ty.name, name)
                    } else {
                        format!(
                            "adds required {}.{}: {} nodes have none",
                            old_ty.name,
                            name,
                            fmt_num(affected)
                        )
                    },
                });
            }
            Field::Edge { field, rel, .. } => {
                changes.push(SchemaChange {
                    severity: Severity::Safe,
                    kind: ChangeKind::RelationshipAdded {
                        type_name: old_ty.name.clone(),
                        field: field.clone(),
                        rel: rel.clone(),
                    },
                    affected: 0,
                    message: format!("adds relationship {}.{} ({})", old_ty.name, field, rel),
                });
            }
        }
    }
}

fn diff_edge_props(
    graph: &Graph,
    type_name: &str,
    field: &str,
    rel: &str,
    old_props: &[EdgeField],
    new_props: &[EdgeField],
    changes: &mut Vec<SchemaChange>,
) {
    let old_map: HashMap<&str, &EdgeField> = old_props.iter().map(|p| (p.name.as_str(), p)).collect();
    let new_map: HashMap<&str, &EdgeField> = new_props.iter().map(|p| (p.name.as_str(), p)).collect();

    for old_prop in old_props {
        if let Some(new_prop) = new_map.get(old_prop.name.as_str()) {
            let old_base = base_type(&old_prop.ty);
            let new_base = base_type(&new_prop.ty);
            if old_base != new_base || !vector_compatible(&old_prop.ty, &new_prop.ty) {
                let affected = count_rels_non_converting(graph, rel, &old_prop.name, &new_prop.ty);
                let severity = if affected > 0 { Severity::Blocks } else { Severity::Warn };
                changes.push(SchemaChange {
                    severity,
                    kind: ChangeKind::EdgePropTypeChanged {
                        type_name: type_name.to_string(),
                        field: field.to_string(),
                        prop: old_prop.name.clone(),
                        from: old_prop.ty.clone(),
                        to: new_prop.ty.clone(),
                    },
                    affected,
                    message: format!(
                        "changes {}.{} edge property {} {} -> {}: {} values don't convert",
                        type_name,
                        field,
                        old_prop.name,
                        old_prop.ty,
                        new_prop.ty,
                        fmt_num(affected)
                    ),
                });
            } else if old_prop.optional && !new_prop.optional {
                let affected = count_rels_missing_prop(graph, rel, &old_prop.name);
                let severity = if affected > 0 { Severity::Blocks } else { Severity::Warn };
                changes.push(SchemaChange {
                    severity,
                    kind: ChangeKind::EdgePropRequired {
                        type_name: type_name.to_string(),
                        field: field.to_string(),
                        prop: old_prop.name.clone(),
                    },
                    affected,
                    message: format!(
                        "makes {}.{} edge property {} required: {} relationships have none",
                        type_name,
                        field,
                        old_prop.name,
                        fmt_num(affected)
                    ),
                });
            } else if !old_prop.optional && new_prop.optional {
                changes.push(SchemaChange {
                    severity: Severity::Safe,
                    kind: ChangeKind::EdgePropRelaxed {
                        type_name: type_name.to_string(),
                        field: field.to_string(),
                        prop: old_prop.name.clone(),
                    },
                    affected: 0,
                    message: format!(
                        "makes {}.{} edge property {} optional",
                        type_name,
                        field,
                        old_prop.name
                    ),
                });
            }
        } else {
            let affected = count_rels_with_prop(graph, rel, &old_prop.name);
            let severity = if affected > 0 { Severity::Blocks } else { Severity::Warn };
            changes.push(SchemaChange {
                severity,
                kind: ChangeKind::EdgePropRemoved {
                    type_name: type_name.to_string(),
                    field: field.to_string(),
                    prop: old_prop.name.clone(),
                },
                affected,
                message: format!(
                    "removes {}.{} edge property {}: {} relationships have it",
                    type_name,
                    field,
                    old_prop.name,
                    fmt_num(affected)
                ),
            });
        }
    }

    for new_prop in new_props {
        if old_map.contains_key(new_prop.name.as_str()) {
            continue;
        }
        let affected = count_rels_missing_prop(graph, rel, &new_prop.name);
        let severity = if new_prop.optional || affected == 0 {
            Severity::Safe
        } else {
            Severity::Blocks
        };
        changes.push(SchemaChange {
            severity,
            kind: ChangeKind::EdgePropAdded {
                type_name: type_name.to_string(),
                field: field.to_string(),
                prop: new_prop.name.clone(),
                required: !new_prop.optional,
            },
            affected,
            message: if new_prop.optional {
                format!(
                    "adds optional {}.{} edge property {}",
                    type_name, field, new_prop.name
                )
            } else {
                format!(
                    "adds required {}.{} edge property {}: {} relationships have none",
                    type_name,
                    field,
                    new_prop.name,
                    fmt_num(affected)
                )
            },
        });
    }
}

fn find_type_rename<'a>(
    old: &'a Schema,
    new: &'a Schema,
    old_types: &HashMap<&str, &'a TypeDef>,
    new_types: &HashMap<&str, &'a TypeDef>,
) -> Option<(&'a str, &'a str)> {
    let removed: Vec<&TypeDef> = old.types.iter().filter(|t| !new_types.contains_key(t.name.as_str())).collect();
    let added: Vec<&TypeDef> = new.types.iter().filter(|t| !old_types.contains_key(t.name.as_str())).collect();
    if removed.len() == 1 && added.len() == 1 && type_signatures_equal(removed[0], added[0]) {
        Some((removed[0].name.as_str(), added[0].name.as_str()))
    } else {
        None
    }
}

fn find_field_rename<'a>(
    _old_ty: &'a TypeDef,
    _new_ty: &'a TypeDef,
    old_fields: &HashMap<&str, &'a Field>,
    new_fields: &HashMap<&str, &'a Field>,
) -> Option<(&'a str, &'a str)> {
    let removed: Vec<&Field> = old_fields.values().filter(|f| !new_fields.contains_key(field_name(f))).copied().collect();
    let added: Vec<&Field> = new_fields.values().filter(|f| !old_fields.contains_key(field_name(f))).copied().collect();
    if removed.len() == 1 && added.len() == 1 {
        if let (Field::Prop { name: from, ty: from_ty, optional: from_opt, .. }, Field::Prop { name: to, ty: to_ty, optional: to_opt, .. }) = (removed[0], added[0]) {
            if base_type(from_ty) == base_type(to_ty) && from_opt == to_opt {
                return Some((from.as_str(), to.as_str()));
            }
        }
    }
    None
}

fn type_signatures_equal(a: &TypeDef, b: &TypeDef) -> bool {
    if a.fields.len() != b.fields.len() {
        return false;
    }
    let b_map: HashMap<&str, &Field> = b.fields.iter().map(|f| (field_name(f), f)).collect();
    for a_field in &a.fields {
        let name = field_name(a_field);
        let Some(b_field) = b_map.get(name) else {
            return false;
        };
        match (a_field, b_field) {
            (
                Field::Prop { ty: a_ty, optional: a_opt, .. },
                Field::Prop { ty: b_ty, optional: b_opt, .. },
            ) => {
                if base_type(a_ty) != base_type(b_ty) || a_opt != b_opt {
                    return false;
                }
            }
            (
                Field::Edge {
                    rel: a_rel,
                    direction: a_dir,
                    targets: a_targets,
                    many: a_many,
                    props: a_props,
                    ..
                },
                Field::Edge {
                    rel: b_rel,
                    direction: b_dir,
                    targets: b_targets,
                    many: b_many,
                    props: b_props,
                    ..
                },
            ) => {
                if a_rel != b_rel || a_dir != b_dir || a_many != b_many {
                    return false;
                }
                if targets_set(a_targets) != targets_set(b_targets) {
                    return false;
                }
                if !edge_props_equal(a_props, b_props) {
                    return false;
                }
            }
            _ => return false,
        }
    }
    true
}

fn edge_props_equal(a: &[EdgeField], b: &[EdgeField]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let b_map: HashMap<&str, &EdgeField> = b.iter().map(|p| (p.name.as_str(), p)).collect();
    for p in a {
        let Some(other) = b_map.get(p.name.as_str()) else {
            return false;
        };
        if base_type(&p.ty) != base_type(&other.ty) || p.optional != other.optional {
            return false;
        }
    }
    true
}

fn targets_set(targets: &[String]) -> HashSet<&str> {
    targets.iter().map(String::as_str).collect()
}

fn field_name(field: &Field) -> &str {
    match field {
        Field::Prop { name, .. } => name,
        Field::Edge { field, .. } => field,
    }
}

fn base_type(ty: &str) -> &str {
    ty.split_once('<').map(|(base, _)| base).unwrap_or(ty)
}

fn vector_compatible(from: &str, to: &str) -> bool {
    if base_type(from) != "Vector" || base_type(to) != "Vector" {
        return true;
    }
    match (VectorSpec::parse(from), VectorSpec::parse(to)) {
        (Some(from_spec), Some(to_spec)) => from_spec.dimensions == to_spec.dimensions,
        _ => false,
    }
}

fn value_converts(value: &Value, target_ty: &str) -> bool {
    let target_base = base_type(target_ty);
    match value {
        Value::String(s) => match target_base {
            "String" => true,
            "Int" => s.trim().parse::<i64>().is_ok(),
            "Float" => s.trim().parse::<f64>().is_ok(),
            "Bool" => matches!(s.trim().to_ascii_lowercase().as_str(), "true" | "false"),
            _ => false,
        },
        Value::Int(_) => matches!(target_base, "Int" | "Float" | "String"),
        Value::Float(_) => matches!(target_base, "Int" | "Float" | "String"),
        Value::Bool(_) => matches!(target_base, "Bool" | "String"),
        Value::Point(_) => target_base == "Point",
        Value::Vector(v) => {
            target_base == "Vector"
                && VectorSpec::parse(target_ty).is_some_and(|spec| spec.dimensions == v.dimensions())
        }
        Value::Null | Value::List(_) | Value::Map(_) => false,
    }
}

fn fmt_num(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out.chars().rev().collect()
}

fn count_nodes_with_label(graph: &Graph, label: &str) -> u64 {
    graph.nodes_by_label(label).map(|set| set.len() as u64).unwrap_or(0)
}

fn count_nodes_missing_prop(graph: &Graph, type_name: &str, prop: &str) -> u64 {
    graph
        .nodes_by_label(type_name)
        .map(|set| {
            set.iter()
                .filter(|&&id| graph.get_node(id).is_some_and(|n| n.prop(prop).is_none()))
                .count() as u64
        })
        .unwrap_or(0)
}

fn count_nodes_with_prop(graph: &Graph, type_name: &str, prop: &str) -> u64 {
    graph
        .nodes_by_label(type_name)
        .map(|set| {
            set.iter()
                .filter(|&&id| graph.get_node(id).is_some_and(|n| n.prop(prop).is_some()))
                .count() as u64
        })
        .unwrap_or(0)
}

fn count_non_converting(graph: &Graph, type_name: &str, prop: &str, target_ty: &str) -> u64 {
    graph
        .nodes_by_label(type_name)
        .map(|set| {
            set.iter()
                .filter(|&&id| {
                    graph.get_node(id).is_some_and(|n| {
                        n.prop(prop)
                            .is_some_and(|v| !matches!(v, Value::Null) && !value_converts(v, target_ty))
                    })
                })
                .count() as u64
        })
        .unwrap_or(0)
}

fn count_rels_with_kind(graph: &Graph, kind: &str) -> u64 {
    graph.relationships().filter(|r| r.kind == kind).count() as u64
}

fn count_rels_missing_prop(graph: &Graph, kind: &str, prop: &str) -> u64 {
    graph
        .relationships()
        .filter(|r| r.kind == kind && r.prop(prop).is_none())
        .count() as u64
}

fn count_rels_with_prop(graph: &Graph, kind: &str, prop: &str) -> u64 {
    graph
        .relationships()
        .filter(|r| r.kind == kind && r.prop(prop).is_some())
        .count() as u64
}

fn count_rels_non_converting(graph: &Graph, kind: &str, prop: &str, target_ty: &str) -> u64 {
    graph
        .relationships()
        .filter(|r| {
            r.kind == kind
                && r.prop(prop)
                    .is_some_and(|v| !matches!(v, Value::Null) && !value_converts(v, target_ty))
        })
        .count() as u64
}

struct EdgeDesc<'a> {
    rel: &'a str,
    direction: Direction,
    targets: &'a [String],
}

fn count_relationships_changed(
    graph: &Graph,
    type_name: &str,
    old: &EdgeDesc<'_>,
    new: &EdgeDesc<'_>,
) -> u64 {
    let rel_changed = old.rel != new.rel;
    let direction_changed = old.direction != new.direction;
    let targets_changed = targets_set(old.targets) != targets_set(new.targets);
    let many_only = !rel_changed && !direction_changed && !targets_changed;
    let new_target_set = targets_set(new.targets);

    graph
        .relationships()
        .filter(|rel| {
            if rel.kind != old.rel {
                return false;
            }
            let (source_id, target_id) = match old.direction {
                Direction::Out => (rel.from, rel.to),
                Direction::In => (rel.to, rel.from),
            };
            let Some(source) = graph.get_node(source_id) else {
                return false;
            };
            if !source.has_label(type_name) {
                return false;
            }
            if rel_changed || direction_changed {
                return true;
            }
            if many_only {
                return false;
            }
            let Some(target) = graph.get_node(target_id) else {
                return true;
            };
            !target.labels().any(|label| new_target_set.contains(label))
        })
        .count() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Zega;

    fn load(zega: &Zega, schema: &str, mutation: &str) {
        zega.run_lang(schema, mutation).unwrap();
    }

    fn find<'a>(changes: &'a [SchemaChange], kind: &str) -> &'a SchemaChange {
        changes
            .iter()
            .find(|c| serde_json::to_value(&c.kind).unwrap()["kind"] == kind)
            .unwrap_or_else(|| panic!("no {kind} change in {changes:?}"))
    }

    #[test]
    fn identical_schemas_are_empty_and_ok() {
        let zega = Zega::in_memory().build().unwrap();
        let schema = "type Player { name: String }";
        let report = zega.schema_diff(schema, schema).unwrap();
        assert!(report.ok);
        assert!(report.changes.is_empty());
    }

    #[test]
    fn empty_old_schema_reports_all_types_added() {
        let zega = Zega::in_memory().build().unwrap();
        let report = zega.schema_diff("", "type Player { name: String }").unwrap();
        assert!(report.ok);
        let added = find(&report.changes, "type_added");
        assert_eq!(added.affected, 0);
    }

    #[test]
    fn populated_type_removal_blocks() {
        let zega = Zega::in_memory().build().unwrap();
        load(&zega, "type Team { name: String }", "mutation { Team(name: \"A\") { name } }");
        load(&zega, "type Team { name: String }", "mutation { Team(name: \"B\") { name } }");
        let report = zega.schema_diff(
            "type Team { name: String }",
            "type Player { name: String salary: Int }",
        ).unwrap();
        let removed = find(&report.changes, "type_removed");
        assert_eq!(removed.severity, Severity::Blocks);
        assert_eq!(removed.affected, 2);
    }

    #[test]
    fn empty_type_removal_warns() {
        let zega = Zega::in_memory().build().unwrap();
        let report = zega.schema_diff(
            "type Team { name: String }",
            "type Player { name: String salary: Int }",
        ).unwrap();
        let removed = find(&report.changes, "type_removed");
        assert_eq!(removed.severity, Severity::Warn);
        assert_eq!(removed.affected, 0);
    }

    #[test]
    fn type_rename_detected_when_signatures_match() {
        let zega = Zega::in_memory().build().unwrap();
        load(&zega, "type Team { name: String }", "mutation { Team(name: \"Flames\") { name } }");
        let report = zega.schema_diff("type Team { name: String }", "type Club { name: String }").unwrap();
        let renamed = find(&report.changes, "type_renamed");
        assert_eq!(renamed.severity, Severity::Warn);
        assert_eq!(renamed.affected, 1);
        assert!(!report.changes.iter().any(|c| matches!(c.kind, ChangeKind::TypeRemoved { .. } | ChangeKind::TypeAdded { .. })));
    }

    #[test]
    fn ambiguous_rename_falls_back_to_remove_and_add() {
        let zega = Zega::in_memory().build().unwrap();
        let report = zega.schema_diff(
            "type Team { name: String } type Squad { name: String }",
            "type Club { name: String } type Crew { name: String }",
        ).unwrap();
        assert_eq!(report.changes.iter().filter(|c| matches!(c.kind, ChangeKind::TypeRemoved { .. })).count(), 2);
        assert_eq!(report.changes.iter().filter(|c| matches!(c.kind, ChangeKind::TypeAdded { .. })).count(), 2);
        assert!(!report.changes.iter().any(|c| matches!(c.kind, ChangeKind::TypeRenamed { .. })));
    }

    #[test]
    fn required_field_added_blocks_when_missing() {
        let zega = Zega::in_memory().build().unwrap();
        load(&zega, "type Player { name: String }", "mutation { Player(name: \"A\") { name } }");
        load(&zega, "type Player { name: String }", "mutation { Player(name: \"B\") { name } }");
        let report = zega.schema_diff("type Player { name: String }", "type Player { name: String email: String }").unwrap();
        let added = find(&report.changes, "field_added");
        assert_eq!(added.severity, Severity::Blocks);
        assert_eq!(added.affected, 2);
    }

    #[test]
    fn required_field_added_warns_when_no_nodes_are_missing() {
        let zega = Zega::in_memory().build().unwrap();
        let report = zega.schema_diff(
            "type Player { name: String email: String }",
            "type Player { name: String email: String phone: String }",
        ).unwrap();
        let added = find(&report.changes, "field_added");
        assert_eq!(added.severity, Severity::Warn);
        assert_eq!(added.affected, 0);
    }

    #[test]
    fn optional_field_added_is_safe() {
        let zega = Zega::in_memory().build().unwrap();
        load(&zega, "type Player { name: String }", "mutation { Player(name: \"A\") { name } }");
        let report = zega.schema_diff("type Player { name: String }", "type Player { name: String nickname?: String }").unwrap();
        let added = find(&report.changes, "field_added");
        assert_eq!(added.severity, Severity::Safe);
    }

    #[test]
    fn required_to_optional_is_safe() {
        let zega = Zega::in_memory().build().unwrap();
        let report = zega.schema_diff("type Player { name: String email: String }", "type Player { name: String email?: String }").unwrap();
        let relaxed = find(&report.changes, "field_relaxed");
        assert_eq!(relaxed.severity, Severity::Safe);
        assert_eq!(relaxed.affected, 0);
    }

    #[test]
    fn optional_to_required_blocks_with_exact_missing_count() {
        let zega = Zega::in_memory().build().unwrap();
        load(&zega, "type Player { name: String email?: String }", "mutation { Player(name: \"A\" && email: \"a@x\") { name } }");
        load(&zega, "type Player { name: String email?: String }", "mutation { Player(name: \"B\") { name } }");
        load(&zega, "type Player { name: String email?: String }", "mutation { Player(name: \"C\") { name } }");
        let report = zega.schema_diff("type Player { name: String email?: String }", "type Player { name: String email: String }").unwrap();
        let required = find(&report.changes, "field_required");
        assert_eq!(required.severity, Severity::Blocks);
        assert_eq!(required.affected, 2);
    }

    #[test]
    fn type_change_with_all_converting_values_warns() {
        let zega = Zega::in_memory().build().unwrap();
        load(&zega, "type Player { name: String score: Int }", "mutation { Player(name: \"A\" && score: 10) { name } }");
        load(&zega, "type Player { name: String score: Int }", "mutation { Player(name: \"B\" && score: 20) { name } }");
        load(&zega, "type Player { name: String score: Int }", "mutation { Player(name: \"C\" && score: 30) { name } }");
        load(&zega, "type Player { name: String score: Int }", "mutation { Player(name: \"D\" && score: 40) { name } }");
        let report = zega.schema_diff("type Player { name: String score: Int }", "type Player { name: String score: String }").unwrap();
        let changed = find(&report.changes, "field_type_changed");
        assert_eq!(changed.severity, Severity::Warn);
        assert_eq!(changed.affected, 0);
    }

    #[test]
    fn int_to_float_fully_convertible_warns() {
        let zega = Zega::in_memory().build().unwrap();
        load(&zega, "type Player { name: String salary: Int }", "mutation { Player(name: \"A\" && salary: 100) { name } }");
        load(&zega, "type Player { name: String salary: Int }", "mutation { Player(name: \"B\" && salary: 200) { name } }");
        let report = zega.schema_diff("type Player { name: String salary: Int }", "type Player { name: String salary: Float }").unwrap();
        let changed = find(&report.changes, "field_type_changed");
        assert_eq!(changed.severity, Severity::Warn);
        assert_eq!(changed.affected, 0);
    }

    #[test]
    fn string_to_int_blocks_on_unparseable_values() {
        let zega = Zega::in_memory().build().unwrap();
        load(&zega, "type Player { name: String year: String }", "mutation { Player(name: \"A\" && year: \"2024\") { name } }");
        load(&zega, "type Player { name: String year: String }", "mutation { Player(name: \"B\" && year: \"nope\") { name } }");
        load(&zega, "type Player { name: String year: String }", "mutation { Player(name: \"C\" && year: \"x\") { name } }");
        let report = zega.schema_diff("type Player { name: String year: String }", "type Player { name: String year: Int }").unwrap();
        let changed = find(&report.changes, "field_type_changed");
        assert_eq!(changed.severity, Severity::Blocks);
        assert_eq!(changed.affected, 2);
    }

    #[test]
    fn field_rename_detected() {
        let zega = Zega::in_memory().build().unwrap();
        load(&zega, "type Player { name: String email: String }", "mutation { Player(name: \"A\" && email: \"a@x\") { name } }");
        let report = zega.schema_diff("type Player { name: String email: String }", "type Player { name: String mailbox: String }").unwrap();
        let renamed = find(&report.changes, "field_renamed");
        assert_eq!(renamed.severity, Severity::Warn);
        assert_eq!(renamed.affected, 1);
    }

    #[test]
    fn relationship_removed_blocks_when_rels_exist() {
        let zega = Zega::in_memory().build().unwrap();
        let schema = "type Player { name: String playsFor: MEMBER -> Team[] } type Team { name: String }";
        load(&zega, schema, "mutation { Player(name: \"A\") { name } }");
        load(&zega, schema, "mutation { Player(name: \"B\") { name } }");
        load(&zega, schema, "mutation { Team(name: \"T\") { name } }");
        let graph = zega.graph_json().unwrap();
        let nodes = graph["nodes"].as_array().unwrap();
        let player_a = nodes.iter().find(|n| n["name"] == "A").unwrap()["id"].as_u64().unwrap();
        let player_b = nodes.iter().find(|n| n["name"] == "B").unwrap()["id"].as_u64().unwrap();
        let team = nodes.iter().find(|n| n["name"] == "T").unwrap()["id"].as_u64().unwrap();
        zega.connect_schema(schema, player_a, "playsFor", team).unwrap();
        zega.connect_schema(schema, player_b, "playsFor", team).unwrap();
        let report = zega.schema_diff(schema, "type Player { name: String } type Team { name: String }").unwrap();
        let removed = find(&report.changes, "relationship_removed");
        assert_eq!(removed.severity, Severity::Blocks);
        assert_eq!(removed.affected, 2);
    }

    #[test]
    fn target_narrowing_counts_violating_rels() {
        let zega = Zega::in_memory().build().unwrap();
        let old_schema = "type Player { name: String } type Team { name: String playsFor: MEMBER -> Player[] }";
        let new_schema = "type Player { name: String } type Team { name: String playsFor: MEMBER -> Team[] }";
        load(&zega, old_schema, "mutation { Player(name: \"A\") { name } }");
        load(&zega, old_schema, "mutation { Team(name: \"T\") { name } }");
        let graph = zega.graph_json().unwrap();
        let player = graph["nodes"].as_array().unwrap().iter().find(|n| n["name"] == "A").unwrap()["id"].as_u64().unwrap();
        let team = graph["nodes"].as_array().unwrap().iter().find(|n| n["name"] == "T").unwrap()["id"].as_u64().unwrap();
        zega.connect_schema(old_schema, team, "playsFor", player).unwrap();
        let report = zega.schema_diff(old_schema, new_schema).unwrap();
        let changed = find(&report.changes, "relationship_changed");
        assert_eq!(changed.severity, Severity::Blocks);
        assert_eq!(changed.affected, 1);
    }

    #[test]
    fn edge_prop_changes_count_affected_rels() {
        let zega = Zega::in_memory().build().unwrap();
        let schema = "type Player { name: String playsFor: MEMBER -> Team[] { since?: Int } } type Team { name: String }";
        load(&zega, schema, "mutation { Player(name: \"A\") { name } }");
        load(&zega, schema, "mutation { Player(name: \"B\") { name } }");
        load(&zega, schema, "mutation { Team(name: \"T\") { name } }");
        let graph = zega.graph_json().unwrap();
        let nodes = graph["nodes"].as_array().unwrap();
        let player_a = nodes.iter().find(|n| n["name"] == "A").unwrap()["id"].as_u64().unwrap();
        let player_b = nodes.iter().find(|n| n["name"] == "B").unwrap()["id"].as_u64().unwrap();
        let team = nodes.iter().find(|n| n["name"] == "T").unwrap()["id"].as_u64().unwrap();
        zega.connect_schema(schema, player_a, "playsFor", team).unwrap();
        zega.connect_schema(schema, player_b, "playsFor", team).unwrap();

        let new_schema = "type Player { name: String playsFor: MEMBER -> Team[] { since: Int } } type Team { name: String }";
        let report = zega.schema_diff(schema, new_schema).unwrap();
        let required = find(&report.changes, "edge_prop_required");
        assert_eq!(required.severity, Severity::Blocks);
        assert_eq!(required.affected, 2);
    }

    #[test]
    fn thousands_separator_in_messages() {
        let zega = Zega::in_memory().build().unwrap();
        let schema = "type Player { name: String }";
        for i in 0..1204 {
            load(&zega, schema, &format!("mutation {{ Player(name: \"p{i}\") {{ name }} }}"));
        }
        let report = zega.schema_diff(schema, "type Other { name: String salary: Int }").unwrap();
        let removed = find(&report.changes, "type_removed");
        assert!(removed.message.contains("1,204"), "{}", removed.message);
    }
}
