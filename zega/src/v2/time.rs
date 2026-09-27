//! Temporal predicates reuse the existing Boolean and #107 chain evaluator.
use super::*;

pub(super) fn property<'a>(
    graph: &'a Graph,
    id: NodeId,
    field: &str,
    at: Option<i64>,
) -> Result<Option<&'a Value>, LangError> {
    if let Some(at) = at {
        if let Some(history) = graph
            .history
            .get()
            .map_err(LangError::bare)?
            .get(&(id, field.to_string()))
        {
            return Ok(history.at(at));
        }
    }
    Ok(graph.get_node(id).and_then(|node| node.prop(field)))
}

// A monotone scalar comparison can be decided from the stored bounds.
// Mixed types and nulls deliberately fall back to the ordinary evaluator.
fn summary(
    graph: &Graph,
    schema: &Schema,
    id: NodeId,
    test: &BoolExpr,
) -> Result<Option<(bool, i64, i64)>, LangError> {
    let BoolExpr::Test(pred) = test else {
        return Ok(None);
    };
    let (field, cmp, value) = match pred {
        Pred::Cmp(field, cmp, value, _) => (field, *cmp, value),
        _ => return Ok(None),
    };
    if graph.get_node(id).is_some_and(|node| {
        schema
            .types
            .iter()
            .any(|ty| node.has_label(&ty.name) && (ty.appears.is_some() || ty.ends.is_some()))
    }) {
        return Ok(None);
    }
    let histories = graph.history.get().map_err(LangError::bare)?;
    if histories.appears.contains_key(&id) || histories.ends.contains_key(&id) {
        return Ok(None);
    }
    let Some(h) = histories.get(&(id, field.clone())).filter(|h| h.ordered) else {
        return Ok(None);
    };
    let low = cmp_json(&value_to_json(&h.min), cmp, value);
    let high = cmp_json(&value_to_json(&h.max), cmp, value);
    Ok((low == high).then_some((low, h.first, h.last)))
}

fn times(
    graph: &Graph,
    schema: &Schema,
    id: NodeId,
    test: &BoolExpr,
) -> Result<Vec<i64>, LangError> {
    // A chain can observe fields, relationship endpoints and lifetimes anywhere
    // along its path. Evaluate complete states at their change boundaries.
    let tests = test.tests();
    let chains = tests
        .iter()
        .any(|p| matches!(p, Pred::Chain(_) | Pred::Ever(..) | Pred::Time(..)));
    let histories = graph.history.get().map_err(LangError::bare)?;
    let mut times: Vec<_> = if chains {
        histories
            .values()
            .flat_map(|h| h.changes.iter().map(|c| c.0))
            .collect()
    } else {
        histories
            .range((id, String::new())..)
            .take_while(|((node, _), _)| *node == id)
            .filter(|((_, field), _)| tests.iter().any(|p| p.field() == field))
            .flat_map(|(_, h)| h.changes.iter().map(|c| c.0))
            .collect()
    };
    if chains {
        times.extend(
            histories
                .relationships
                .values()
                .flat_map(|h| std::iter::once(h.from).chain(h.to)),
        );
        times.extend(
            histories
                .appears
                .values()
                .chain(histories.ends.values())
                .copied(),
        );
    } else {
        times.extend(
            histories
                .appears
                .get(&id)
                .into_iter()
                .chain(histories.ends.get(&id))
                .copied(),
        );
    }
    if chains {
        for node in graph.nodes() {
            let (from, to) = lifetime(graph, schema, histories, node.id);
            times.extend(from.into_iter().chain(to));
        }
    } else {
        let (from, to) = lifetime(graph, schema, histories, id);
        times.extend(from.into_iter().chain(to));
    }
    times.retain(|t| *t != i64::MIN && visible_with(graph, schema, histories, id, *t));
    times.sort_unstable();
    times.dedup();
    Ok(times)
}
fn at(
    graph: &Graph,
    schema: &Schema,
    id: NodeId,
    test: &BoolExpr,
    t: i64,
    work: &mut Work,
) -> Result<bool, LangError> {
    let old = work.at.replace(t);
    // A pinned set is only reusable at the time it was computed.
    let pins = std::mem::take(&mut work.pins);
    let result = node_matches(graph, schema, id, Some(test), work);
    work.at = old;
    work.pins = pins;
    result
}
pub(super) fn first_last(
    graph: &Graph,
    schema: &Schema,
    id: NodeId,
    test: &BoolExpr,
    last: bool,
    work: &mut Work,
) -> Result<Option<i64>, LangError> {
    if let Some((matches, first, end)) = summary(graph, schema, id, test)? {
        return Ok(matches.then_some(if last { end } else { first }));
    }
    let mut dates = times(graph, schema, id, test)?;
    if last {
        dates.reverse();
    }
    for t in dates {
        work.step()?;
        if at(graph, schema, id, test, t, work)? {
            return Ok(Some(t));
        }
    }
    Ok(None)
}
pub(super) fn ever_always(
    graph: &Graph,
    schema: &Schema,
    id: NodeId,
    test: &BoolExpr,
    always: bool,
    window: Option<crate::lang::TimeWindow>,
    work: &mut Work,
) -> Result<bool, LangError> {
    if window.is_none() {
        if let Some((matches, _, _)) = summary(graph, schema, id, test)? {
            return Ok(matches);
        }
    }
    let dates = if let Some(window) = window {
        boundaries(graph, schema, window)?
    } else {
        times(graph, schema, id, test)?
    };
    if dates.is_empty() {
        return Ok(false);
    }
    for t in dates {
        work.step()?;
        if at(graph, schema, id, test, t, work)? != always {
            return Ok(!always);
        }
    }
    Ok(always)
}
pub(super) fn series(
    graph: &Graph,
    id: NodeId,
    field: &str,
    from: i64,
    to: i64,
    unit: &str,
    work: &mut Work,
) -> Result<Json, LangError> {
    let h = graph.history.get().map_err(LangError::bare)?;
    let history = h.get(&(id, field.to_string()));
    let mut rows = Vec::new();
    let mut t = from;
    while t <= to {
        work.step()?;
        rows.push(json!({"time":crate::history::format_date(t),"value":history.and_then(|h|h.at(t)).map(value_to_json).unwrap_or(Json::Null)}));
        t = if unit == "month" {
            let date = crate::history::format_date(t);
            let year: i64 = date[..4]
                .parse()
                .map_err(|_| LangError::bare("series year out of range"))?;
            let month: i64 = date[5..7].parse().expect("formatted month");
            let (year, month) = if month == 12 {
                (year + 1, 1)
            } else {
                (year, month + 1)
            };
            // Calendar-month samples are the first of the next month.
            crate::history::date(&format!("{year:04}-{month:02}-01")).map_err(LangError::bare)?
        } else {
            t.checked_add(if unit == "week" { 7 * 86400 } else { 86400 })
                .ok_or_else(|| LangError::bare("series time overflow"))?
        };
    }
    Ok(Json::Array(rows))
}

fn lifetime(
    graph: &Graph,
    schema: &Schema,
    h: &crate::history::Histories,
    id: NodeId,
) -> (Option<i64>, Option<i64>) {
    let mut bounds = (h.appears.get(&id).copied(), h.ends.get(&id).copied());
    if let Some(node) = graph.get_node(id) {
        for ty in &schema.types {
            if !node.has_label(&ty.name) {
                continue;
            }
            for (bound, field) in [(&mut bounds.0, &ty.appears), (&mut bounds.1, &ty.ends)] {
                if bound.is_none() {
                    *bound =
                        field
                            .as_ref()
                            .and_then(|field| node.prop(field))
                            .and_then(|v| match v {
                                Value::String(s) => crate::history::date(s).ok(),
                                _ => None,
                            });
                }
            }
        }
    }
    bounds
}
fn visible_with(
    graph: &Graph,
    schema: &Schema,
    h: &crate::history::Histories,
    id: NodeId,
    at: i64,
) -> bool {
    let (from, to) = lifetime(graph, schema, h, id);
    h.visible(id, at) && from.is_none_or(|from| from <= at) && to.is_none_or(|to| at < to)
}
pub(super) fn visible(
    graph: &Graph,
    schema: &Schema,
    id: NodeId,
    at: Option<i64>,
) -> Result<bool, LangError> {
    let Some(at) = at else {
        return Ok(true);
    };
    Ok(visible_with(
        graph,
        schema,
        graph.history.get().map_err(LangError::bare)?,
        id,
        at,
    ))
}
pub(super) fn neighbors_at(
    graph: &Graph,
    schema: &Schema,
    id: NodeId,
    kind: &str,
    direction: Direction,
    at: Option<i64>,
) -> Result<Vec<(NodeId, RelId)>, LangError> {
    let Some(at) = at else {
        return Ok(neighbors(graph, id, kind, direction));
    };
    let h = graph.history.get().map_err(LangError::bare)?;
    if !visible_with(graph, schema, h, id, at) {
        return Ok(Vec::new());
    }
    let mut out = neighbors(graph, id, kind, direction);
    out.retain(|(next, rel)| {
        !h.relationships.contains_key(rel) && visible_with(graph, schema, h, *next, at)
    });
    for entry in h.relationships.values() {
        let r = &entry.rel;
        if r.kind != kind || !entry.contains(at) {
            continue;
        }
        let next = match direction {
            Direction::Out if r.from == id => r.to,
            Direction::In if r.to == id => r.from,
            _ => continue,
        };
        if graph.get_node(next).is_some() && visible_with(graph, schema, h, next, at) {
            out.push((next, r.id));
        }
    }
    out.sort_unstable();
    Ok(out)
}

fn boundaries(
    graph: &Graph,
    schema: &Schema,
    window: crate::lang::TimeWindow,
) -> Result<Vec<i64>, LangError> {
    if window.season {
        return Err(LangError::bare(
            "APS 24 does not define season boundaries for YYYY-YY; use during <date> to <date>",
        ));
    }
    let h = graph.history.get().map_err(LangError::bare)?;
    let mut dates = vec![window.from, window.to];
    if !window.changes {
        for history in h.values() {
            let first = history.changes.partition_point(|c| c.0 <= window.from);
            let last = history.changes.partition_point(|c| c.0 < window.to);
            if first < last {
                dates.extend(history.changes[first..last].iter().map(|c| c.0));
            }
        }
        for node in graph.nodes() {
            let (from, to) = lifetime(graph, schema, h, node.id);
            dates.extend(
                from.into_iter()
                    .chain(to)
                    .filter(|t| *t > window.from && *t < window.to),
            );
        }
        dates.extend(
            h.relationships
                .values()
                .flat_map(|h| std::iter::once(h.from).chain(h.to))
                .filter(|t| *t > window.from && *t < window.to),
        );
        dates.extend(
            h.appears
                .values()
                .chain(h.ends.values())
                .copied()
                .filter(|t| *t > window.from && *t < window.to),
        );
    }
    dates.sort_unstable();
    dates.dedup();
    Ok(dates)
}
#[derive(Clone, Copy)]
pub(super) struct WindowWalk<'a> {
    pub id: NodeId,
    pub field: &'a str,
    pub direction: Direction,
    pub range: Option<(usize, usize)>,
    pub span: Span,
    pub hops: usize,
}
pub(super) fn window_read(
    graph: &Graph,
    schema: &Schema,
    selection: &Selection,
    parent: Option<WindowWalk<'_>>,
    window: crate::lang::TimeWindow,
    context: &mut ReadContext<'_>,
) -> Result<Json, LangError> {
    let mut selection = selection.clone();
    selection.window = None;
    let old_at = context.work.at;
    let pins = std::mem::take(&mut context.work.pins);
    let result = (|| {
        let dates = boundaries(graph, schema, window)?;
        let mut first = std::collections::BTreeMap::new();
        let mut last = std::collections::BTreeMap::new();
        let mut union = std::collections::BTreeMap::new();
        for (index, at) in dates.iter().enumerate() {
            context.work.step()?;
            context.work.at = Some(*at);
            context.work.pins.clear();
            let rows: Vec<(NodeId, usize, Option<RelId>)> = if let Some(WindowWalk {
                id,
                field,
                direction,
                range,
                span,
                ..
            }) = parent
            {
                let node = graph
                    .get_node(id)
                    .ok_or_else(|| LangError::bare("missing window parent"))?;
                let ty = node
                    .first_label()
                    .ok_or_else(|| LangError::bare("missing window parent type"))?;
                let (_, kind, declared, targets, many) =
                    schema.edge(ty, field)?.as_edge().expect("edge");
                if direction != declared {
                    return Err(LangError::bare("relationship does not point that way"));
                }
                if let Some(range) = range {
                    walk_range(
                        graph,
                        schema,
                        id,
                        WalkSpec {
                            rel: kind,
                            field,
                            direction,
                            targets,
                            range,
                            single_valued: !many,
                            span,
                        },
                        context.work,
                    )?
                    .into_iter()
                    .map(|(id, depth, rel)| (id, depth, Some(rel)))
                    .collect()
                } else {
                    neighbors_at(graph, schema, id, kind, direction, Some(*at))?
                        .into_iter()
                        .map(|(id, rel)| (id, 1, Some(rel)))
                        .collect()
                }
            } else {
                candidates(graph, schema, &selection, context.work)?
                    .into_iter()
                    .map(|id| (id, 0, None))
                    .collect()
            };
            let mut rows: Vec<_> = rows
                .into_iter()
                .filter(|(id, ..)| {
                    node_has_any_label(
                        graph,
                        *id,
                        &std::iter::once(selection.type_name.clone())
                            .chain(selection.also.clone())
                            .collect::<Vec<_>>(),
                    )
                })
                .collect();
            let mut kept = Vec::new();
            for row in rows.drain(..) {
                if node_matches(
                    graph,
                    schema,
                    row.0,
                    selection.condition.as_ref(),
                    context.work,
                )? {
                    kept.push(row);
                }
            }
            order_limit(graph, &selection, &mut kept, |r| r.0, context.work)?;
            let mut snapshot = std::collections::BTreeMap::new();
            for (id, depth, rel) in kept {
                snapshot.insert(
                    id,
                    project(
                        graph,
                        schema,
                        &selection,
                        id,
                        parent.map_or(0, |p| p.hops) + depth,
                        rel,
                        context,
                    )?,
                );
            }
            for (id, row) in &snapshot {
                union.entry(*id).or_insert_with(|| row.clone());
            }
            if index == 0 {
                first = snapshot.clone();
            }
            last = snapshot;
        }
        if !window.changes {
            return Ok(Json::Array(union.into_values().collect()));
        }
        let joined: Vec<_> = last
            .iter()
            .filter(|(id, _)| !first.contains_key(id))
            .map(|(_, row)| row.clone())
            .collect();
        let left: Vec<_> = first
            .iter()
            .filter(|(id, _)| !last.contains_key(id))
            .map(|(_, row)| row.clone())
            .collect();
        let changed: Vec<_> = first
            .iter()
            .filter_map(|(id, before)| {
                last.get(id)
                    .filter(|after| *after != before)
                    .map(|after| json!({"id":id,"from":before,"to":after}))
            })
            .collect();
        Ok(json!({"joined":joined,"left":left,"changed":changed}))
    })();
    context.work.at = old_at;
    context.work.pins = pins;
    result
}

pub(super) fn edge_property<'a>(
    graph: &'a Graph,
    id: RelId,
    field: &str,
    at: Option<i64>,
) -> Result<Option<&'a Value>, LangError> {
    if let Some(at) = at {
        if let Some(h) = graph
            .history
            .get()
            .map_err(LangError::bare)?
            .relationships
            .get(&id)
        {
            return Ok(h.contains(at).then(|| h.rel.props.get(field)).flatten());
        }
    }
    Ok(graph.get_relationship(id).and_then(|r| r.prop(field)))
}
pub(super) fn edge_json(
    graph: &Graph,
    id: RelId,
    at: Option<i64>,
) -> Result<Option<Json>, LangError> {
    if let Some(at) = at {
        if let Some(h) = graph
            .history
            .get()
            .map_err(LangError::bare)?
            .relationships
            .get(&id)
        {
            let r = &h.rel;
            let props: serde_json::Map<_, _> = r
                .props
                .iter()
                .map(|(k, v)| (k.clone(), value_to_json(v)))
                .collect();
            return Ok(h
                .contains(at)
                .then(|| json!({"id":r.id,"type":r.kind,"from":r.from,"to":r.to,"props":props})));
        }
    }
    Ok(graph.get_relationship(id).map(rel_json))
}
