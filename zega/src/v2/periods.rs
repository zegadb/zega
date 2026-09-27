//! Resolve named periods from the live graph once per statement, before reading history.
use super::*;
use crate::lang::{Field, Period, TimeClause, TimeDate, TimeWindow, TypeDef};

fn calendar<'a>(
    schema: &'a Schema,
    ty: &str,
    word: &str,
) -> Result<(&'a TypeDef, &'a Period), LangError> {
    let owner = schema.get(ty)?;
    let target = owner.calendars.get(word).ok_or_else(|| {
        LangError::bare(format!(
            "{ty} has no calendar {word}; add `calendar {word} -> PeriodType` inside type {ty}"
        ))
    })?;
    let target = schema.get(target).map_err(|_| {
        LangError::bare(format!(
            "calendar {word} points at unknown type {target}; declare that period type"
        ))
    })?;
    let period = target.period.as_ref().ok_or_else(|| LangError::bare(format!(
        "calendar {word} points at {} without a period; add `period from <Date field> to <Date field> named by <field>` inside type {}", target.name, target.name
    )))?;
    Ok((target, period))
}

fn bounds(node: &impl NodeView, ty: &TypeDef, period: &Period) -> Result<(i64, i64), LangError> {
    let date = |field: &str| {
        node.prop(field)
            .and_then(Value::as_string)
            .ok_or_else(|| {
                LangError::bare(format!(
                    "period {} needs a Date value for {field}; set the node's {field}",
                    ty.name
                ))
            })
            .and_then(|s| crate::history::date(s).map_err(LangError::bare))
    };
    let (from, to) = (date(&period.from)?, date(&period.to)?);
    if from > to {
        return Err(LangError::bare(format!(
            "period {} starts after it ends; fix {} and {}",
            ty.name, period.from, period.to
        )));
    }
    Ok((from, to))
}

fn year_name(name: &Json, word: &str) -> Result<Json, LangError> {
    if let Some(year) = name.as_i64().filter(|y| (0..100).contains(y)) {
        return Ok(json!(2000 + year));
    }
    let text = name
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| name.to_string());
    let parts: Vec<_> = text.split('-').collect();
    let year = |s: &str| -> Option<i64> {
        if !matches!(s.len(), 2 | 4) || !s.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        s.parse::<i64>()
            .ok()
            .map(|y| if s.len() == 2 { 2000 + y } else { y })
    };
    let invalid = || {
        LangError::bare(format!(
            "invalid {word} name {text}; use 26, 2026, or consecutive years 26-27"
        ))
    };
    let first = year(parts[0]).ok_or_else(invalid)?;
    if parts.len() > 2 || (parts.len() == 2 && year(parts[1]) != Some(first + 1)) {
        return Err(invalid());
    }
    Ok(json!(first))
}

fn resolve(
    date: &mut TimeDate,
    graph: &Graph,
    schema: &Schema,
    ty: &str,
    work: &mut Work,
) -> Result<(), LangError> {
    let at = match date {
        TimeDate::Instant(_) => return Ok(()),
        TimeDate::Year(year, end) => {
            let y = *year + i32::from(*end);
            crate::history::date(&format!("{y:04}-01-01")).map_err(LangError::bare)?
                - i64::from(*end)
        }
        TimeDate::Period(period_name) => {
            let (word, name, end) = (&period_name.word, &period_name.name, period_name.end);
            let (target, period) = calendar(schema, ty, word)?;
            let year = target.fields.iter().any(|f| matches!(f, Field::Prop { name, ty, .. } if name == &period.named && ty == "Int"));
            let key = if year {
                year_name(name, word)?
            } else {
                name.clone()
            };
            let mut found = None;
            for node in graph
                .nodes_by_label(&target.name)
                .into_iter()
                .flat_map(|ids| ids.iter())
                .filter_map(|id| graph.get_node(*id))
            {
                work.step()?;
                if node
                    .prop(&period.named)
                    .is_some_and(|v| value_to_json(v) == key)
                {
                    if found.is_some() {
                        return Err(LangError::bare(format!(
                            "multiple {word} {key} nodes in {}; give each period a unique {}",
                            target.name, period.named
                        )));
                    }
                    found = Some(bounds(&node, target, period)?);
                }
            }
            let (from, to) = found.ok_or_else(|| {
                LangError::bare(format!(
                    "no {word} {key} in {}; add a {} node with {} = {key} and its actual dates",
                    target.name, target.name, period.named
                ))
            })?;
            if end {
                to
            } else {
                from
            }
        }
    };
    *date = TimeDate::Instant(at);
    Ok(())
}

fn window(
    w: &mut TimeWindow,
    graph: &Graph,
    schema: &Schema,
    ty: &str,
    work: &mut Work,
) -> Result<(), LangError> {
    resolve(&mut w.from, graph, schema, ty, work)?;
    resolve(&mut w.to, graph, schema, ty, work)?;
    if w.from.instant()? > w.to.instant()? {
        return Err(LangError::bare(
            "window starts after it ends; reverse the endpoints",
        ));
    }
    Ok(())
}

fn expression(
    expr: &mut BoolExpr,
    graph: &Graph,
    schema: &Schema,
    ty: &str,
    work: &mut Work,
) -> Result<(), LangError> {
    match expr {
        BoolExpr::And(items) | BoolExpr::Or(items) => {
            for item in items {
                expression(item, graph, schema, ty, work)?;
            }
        }
        BoolExpr::Test(Pred::Ever(_, test, span, _)) => {
            if let Some(w) = span {
                window(w, graph, schema, ty, work)?;
            }
            expression(test, graph, schema, ty, work)?;
        }
        BoolExpr::Test(Pred::Time(_, test, _, date, _)) => {
            resolve(date, graph, schema, ty, work)?;
            expression(test, graph, schema, ty, work)?;
        }
        BoolExpr::Test(Pred::Chain(chain)) => {
            let mut current = ty.to_owned();
            for hop in &mut chain.hops {
                if let Some(target) = schema.get(&current)?.fields.iter().find_map(|f| match f {
                    Field::Edge { field, targets, .. } if field == &hop.field => targets.first(),
                    _ => None,
                }) {
                    current = target.clone();
                }
                if let Some(test) = &mut hop.test {
                    expression(test, graph, schema, &current, work)?;
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn selection(
    sel: &mut Selection,
    graph: &Graph,
    schema: &Schema,
    work: &mut Work,
) -> Result<(), LangError> {
    if let Some(w) = &mut sel.window {
        window(w, graph, schema, &sel.type_name, work)?;
    }
    if let Some(test) = &mut sel.condition {
        expression(test, graph, schema, &sel.type_name, work)?;
    }
    for item in &mut sel.items {
        match item {
            Item::Walk { target, .. } => selection(target, graph, schema, work)?,
            Item::Time(_, _, test, _) => expression(test, graph, schema, &sel.type_name, work)?,
            _ => {}
        }
    }
    Ok(())
}

pub(super) fn query(
    query: &mut crate::lang::Query,
    graph: &Graph,
    schema: &Schema,
    work: &mut Work,
) -> Result<(), LangError> {
    let Some(root) = &mut query.root else {
        return Ok(());
    };
    work.series = None;
    if let Some(time) = &mut query.time {
        match time {
            TimeClause::AsOf(date) => resolve(date, graph, schema, &root.type_name, work)?,
            TimeClause::Window(w) => window(w, graph, schema, &root.type_name, work)?,
            TimeClause::Series { from, to, unit } => {
                resolve(from, graph, schema, &root.type_name, work)?;
                resolve(to, graph, schema, &root.type_name, work)?;
                let (from, to) = (from.instant()?, to.instant()?);
                if from > to {
                    return Err(LangError::bare(
                        "series starts after it ends; reverse the endpoints",
                    ));
                }
                let mut dates = Vec::new();
                if matches!(unit.as_str(), "day" | "week" | "month") {
                    let mut t = from;
                    while t <= to {
                        work.step()?;
                        dates.push(t);
                        t = time::next_sample(t, unit)?;
                    }
                } else {
                    let (target, period) = calendar(schema, &root.type_name, unit)?;
                    for node in graph
                        .nodes_by_label(&target.name)
                        .into_iter()
                        .flat_map(|ids| ids.iter())
                        .filter_map(|id| graph.get_node(*id))
                    {
                        work.step()?;
                        if node.has_label(&target.name) {
                            let (start, end) = bounds(&node, target, period)?;
                            if start <= to && end >= from {
                                dates.push(start);
                            }
                        }
                    }
                    dates.sort_unstable();
                }
                work.series = Some(dates);
            }
        }
    }
    selection(root, graph, schema, work)
}

fn temporal_expr(expr: &BoolExpr) -> bool {
    match expr {
        BoolExpr::And(items) | BoolExpr::Or(items) => items.iter().any(temporal_expr),
        BoolExpr::Test(Pred::Ever(_, test, window, _)) => window.is_some() || temporal_expr(test),
        BoolExpr::Test(Pred::Time(..)) => true,
        BoolExpr::Test(Pred::Chain(chain)) => chain
            .hops
            .iter()
            .any(|hop| hop.test.as_ref().is_some_and(temporal_expr)),
        _ => false,
    }
}
fn temporal_selection(sel: &Selection) -> bool {
    sel.window.is_some()
        || sel.condition.as_ref().is_some_and(temporal_expr)
        || sel.items.iter().any(|item| match item {
            Item::Walk { target, .. } => temporal_selection(target),
            Item::Time(_, _, test, _) => temporal_expr(test),
            _ => false,
        })
}
pub(super) fn prepare<'a>(
    query: &'a crate::lang::Query,
    graph: &Graph,
    schema: &Schema,
    work: &mut Work,
) -> Result<std::borrow::Cow<'a, crate::lang::Query>, LangError> {
    work.series = None;
    if query.time.is_none() && !query.root.as_ref().is_some_and(temporal_selection) {
        return Ok(std::borrow::Cow::Borrowed(query));
    }
    let mut resolved = query.clone();
    self::query(&mut resolved, graph, schema, work)?;
    Ok(std::borrow::Cow::Owned(resolved))
}
