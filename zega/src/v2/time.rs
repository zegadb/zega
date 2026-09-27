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
    let histories = graph.history.get().map_err(LangError::bare)?;
    let Some(h) = histories.get(&(id, field.clone())).filter(|h| h.ordered) else {
        return Ok(None);
    };
    let low = cmp_json(&value_to_json(&h.min), cmp, value);
    let high = cmp_json(&value_to_json(&h.max), cmp, value);
    Ok((low == high).then_some((low, h.first, h.last)))
}

fn times(graph: &Graph, id: NodeId, test: &BoolExpr) -> Result<Vec<i64>, LangError> {
    // Chains can observe a changing field on any reached node. Phase 1 leaves
    // adjacency live; phase 2 will add relationship and lifetime boundaries.
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
    let result = eval_expr(graph, schema, id, test, work);
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
    if let Some((matches, first, end)) = summary(graph, id, test)? {
        return Ok(matches.then_some(if last { end } else { first }));
    }
    let mut dates = times(graph, id, test)?;
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
    work: &mut Work,
) -> Result<bool, LangError> {
    if let Some((matches, _, _)) = summary(graph, id, test)? {
        return Ok(matches);
    }
    let dates = times(graph, id, test)?;
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
