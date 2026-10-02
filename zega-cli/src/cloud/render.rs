//! Human-readable output for `zega cloud`: aligned tables and detail views
//! built from the API's JSON (`--json` skips all of this and prints the API's
//! own bytes).
//!
//! Every string that came from the API is cleaned before it reaches the
//! terminal: a project can be named anything, and a control character in a
//! name must not drive the terminal.

use super::api::CloudError;
use serde_json::Value;

/// `-` for what the API leaves null or out.
const NONE: &str = "-";

/// A string for a terminal: control characters become `?`.
pub fn clean(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { '?' } else { c })
        .collect()
}

/// A field's text: strings as they are, numbers and booleans written out,
/// null or missing as `-`.
fn field(value: &Value, name: &str) -> String {
    match value.get(name) {
        Some(Value::String(text)) => clean(text),
        Some(Value::Number(number)) => number.to_string(),
        Some(Value::Bool(flag)) => flag.to_string(),
        _ => NONE.to_string(),
    }
}

fn number(value: &Value, name: &str) -> Option<f64> {
    value.get(name).and_then(Value::as_f64)
}

fn cents(value: Option<i64>) -> String {
    match value {
        Some(cents) => format!(
            "{}${}.{:02}",
            if cents < 0 { "-" } else { "" },
            cents.abs() / 100,
            cents.abs() % 100
        ),
        None => NONE.to_string(),
    }
}

fn cents_field(value: &Value, name: &str) -> String {
    cents(value.get(name).and_then(Value::as_i64))
}

/// Decimal units, like the API's own gigabytes.
fn bytes(count: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut size = count as f64;
    let mut unit = 0;
    while size >= 1000.0 && unit < UNITS.len() - 1 {
        size /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{count} B")
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}

fn quantity(value: Option<f64>) -> String {
    match value {
        Some(value) if value.fract() == 0.0 => format!("{value:.0}"),
        Some(value) => format!("{value:.2}"),
        None => NONE.to_string(),
    }
}

/// A resource's project as `name (id)`.
fn project(value: &Value) -> String {
    match (
        value.get("projectName").and_then(Value::as_str),
        value.get("projectId").and_then(Value::as_str),
    ) {
        (Some(name), Some(id)) => format!("{} ({})", clean(name), clean(id)),
        (None, Some(id)) => clean(id),
        _ => NONE.to_string(),
    }
}

fn items<'a>(body: &'a Value, key: &str) -> Result<&'a [Value], CloudError> {
    body.get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .ok_or_else(|| {
            CloudError::Local(format!(
                "unexpected answer: no `{key}` list in it (is --api a Zega Cloud address?)"
            ))
        })
}

fn one<'a>(body: &'a Value, key: &str) -> Result<&'a Value, CloudError> {
    body.get(key)
        .filter(|value| value.is_object())
        .ok_or_else(|| {
            CloudError::Local(format!(
                "unexpected answer: no `{key}` in it (is --api a Zega Cloud address?)"
            ))
        })
}

/// An aligned table: header in capitals, two spaces between columns, numbers
/// right-aligned, no trailing spaces.
pub struct Table {
    headers: Vec<&'static str>,
    right: Vec<bool>,
    rows: Vec<Vec<String>>,
}

impl Table {
    pub fn new(columns: &[(&'static str, bool)]) -> Self {
        Self {
            headers: columns.iter().map(|(name, _)| *name).collect(),
            right: columns.iter().map(|(_, right)| *right).collect(),
            rows: Vec::new(),
        }
    }

    pub fn row(&mut self, cells: Vec<String>) {
        debug_assert_eq!(cells.len(), self.headers.len());
        self.rows.push(cells);
    }

    pub fn render(&self) -> String {
        let widths: Vec<usize> = (0..self.headers.len())
            .map(|column| {
                self.rows
                    .iter()
                    .map(|row| row[column].chars().count())
                    .chain([self.headers[column].chars().count()])
                    .max()
                    .unwrap_or(0)
            })
            .collect();
        let line = |cells: &[&str]| {
            let padded: Vec<String> = cells
                .iter()
                .enumerate()
                .map(|(column, cell)| {
                    let pad = widths[column] - cell.chars().count();
                    if self.right[column] {
                        format!("{}{cell}", " ".repeat(pad))
                    } else {
                        format!("{cell}{}", " ".repeat(pad))
                    }
                })
                .collect();
            format!("{}\n", padded.join("  ").trim_end())
        };
        let mut out = line(&self.headers);
        for row in &self.rows {
            out.push_str(&line(&row.iter().map(String::as_str).collect::<Vec<_>>()));
        }
        out
    }
}

/// Labelled lines for one resource.
struct Detail(Vec<(&'static str, String)>);

impl Detail {
    fn render(&self) -> String {
        let width = self
            .0
            .iter()
            .map(|(label, _)| label.len())
            .max()
            .unwrap_or(0);
        self.0
            .iter()
            .map(|(label, value)| format!("{label:<width$}  {value}\n"))
            .collect()
    }
}

/// `api` is shown when the caller has not just said which host it is.
pub fn whoami(body: &Value, api: Option<&str>) -> Result<String, CloudError> {
    let token = one(body, "token")?;
    let prefix = field(token, "prefix");
    let mut rows = Vec::new();
    if let Some(api) = api {
        rows.push(("API", clean(api)));
    }
    rows.extend([
        ("Token", format!("{} ({prefix}...)", field(token, "name"))),
        ("Scope", field(token, "scope")),
        (
            "Project",
            if token.get("projectId").is_some_and(|id| !id.is_null()) {
                project(token)
            } else {
                "all projects".to_string()
            },
        ),
        ("Created", field(token, "createdAt")),
        (
            "Last used",
            token
                .get("lastUsedAt")
                .and_then(Value::as_str)
                .map_or("never".into(), clean),
        ),
        (
            "Expires",
            token
                .get("expiresAt")
                .and_then(Value::as_str)
                .map_or("never".into(), clean),
        ),
    ]);
    Ok(Detail(rows).render())
}

fn graph_rows(graphs: &[Value]) -> Table {
    let mut table = Table::new(&[
        ("ID", false),
        ("NAME", false),
        ("PROJECT", false),
        ("REGION", false),
        ("TIER", false),
        ("STATUS", false),
    ]);
    for graph in graphs {
        table.row(vec![
            field(graph, "id"),
            field(graph, "name"),
            project(graph),
            field(graph, "region"),
            field(graph, "tier"),
            field(graph, "status"),
        ]);
    }
    table
}

pub fn projects(body: &Value) -> Result<String, CloudError> {
    let projects = items(body, "projects")?;
    if projects.is_empty() {
        return Ok("no projects\n".into());
    }
    let mut table = Table::new(&[
        ("ID", false),
        ("NAME", false),
        ("GRAPHS", true),
        ("CREATED", false),
    ]);
    for project in projects {
        let graphs = project
            .get("graphs")
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        table.row(vec![
            field(project, "id"),
            field(project, "name"),
            graphs.to_string(),
            field(project, "createdAt"),
        ]);
    }
    Ok(table.render())
}

pub fn project_one(body: &Value) -> Result<String, CloudError> {
    let project = one(body, "project")?;
    let mut out = Detail(vec![
        ("ID", field(project, "id")),
        ("Name", field(project, "name")),
        ("Created", field(project, "createdAt")),
    ])
    .render();
    let graphs = items(project, "graphs")?;
    if graphs.is_empty() {
        out.push_str("\nno graphs\n");
    } else {
        out.push('\n');
        out.push_str(&graph_rows(graphs).render());
    }
    Ok(out)
}

pub fn graphs(body: &Value) -> Result<String, CloudError> {
    let graphs = items(body, "graphs")?;
    if graphs.is_empty() {
        return Ok("no graphs\n".into());
    }
    Ok(graph_rows(graphs).render())
}

pub fn graph_one(body: &Value) -> Result<String, CloudError> {
    let graph = one(body, "graph")?;
    let mut rows = vec![
        ("ID", field(graph, "id")),
        ("Name", field(graph, "name")),
        ("Project", project(graph)),
        ("Status", field(graph, "status")),
        ("Region", field(graph, "region")),
        (
            "Tier",
            format!("{} ({} MB)", field(graph, "tier"), field(graph, "memoryMb")),
        ),
        (
            "Billing",
            format!(
                "{}, {} base price",
                field(graph, "billing"),
                cents_field(graph, "priceCents")
            ),
        ),
        ("Address", field(graph, "hostUrl")),
        ("Created", field(graph, "createdAt")),
    ];
    for (label, name) in [
        ("Grace until", "graceUntil"),
        ("Data deleted", "deleteAfter"),
    ] {
        if graph.get(name).is_some_and(|value| !value.is_null()) {
            rows.push((label, field(graph, name)));
        }
    }
    Ok(Detail(rows).render())
}

pub fn buckets(body: &Value) -> Result<String, CloudError> {
    let buckets = items(body, "buckets")?;
    if buckets.is_empty() {
        return Ok("no buckets\n".into());
    }
    let mut table = Table::new(&[
        ("ID", false),
        ("NAME", false),
        ("PROJECT", false),
        ("STORED", true),
        ("CREATED", false),
    ]);
    for bucket in buckets {
        table.row(vec![
            field(bucket, "id"),
            field(bucket, "name"),
            project(bucket),
            bucket
                .get("bytesStored")
                .and_then(Value::as_u64)
                .map_or(NONE.into(), bytes),
            field(bucket, "createdAt"),
        ]);
    }
    Ok(table.render())
}

pub fn bucket_one(body: &Value) -> Result<String, CloudError> {
    let bucket = one(body, "bucket")?;
    Ok(Detail(vec![
        ("ID", field(bucket, "id")),
        ("Name", field(bucket, "name")),
        ("Project", project(bucket)),
        ("Address", field(bucket, "url")),
        (
            "Stored",
            bucket
                .get("bytesStored")
                .and_then(Value::as_u64)
                .map_or(NONE.into(), bytes),
        ),
        (
            "Cap",
            format!(
                "{} of overage a month (changed in the dashboard)",
                cents_field(bucket, "capCents")
            ),
        ),
        ("Created", field(bucket, "createdAt")),
    ])
    .render())
}

fn deployed(function: &Value) -> String {
    if function.get("deployed").and_then(Value::as_bool) == Some(true) {
        format!("v{}", field(function, "version"))
    } else {
        "no".to_string()
    }
}

pub fn functions(body: &Value) -> Result<String, CloudError> {
    let functions = items(body, "functions")?;
    if functions.is_empty() {
        return Ok("no functions\n".into());
    }
    let mut table = Table::new(&[
        ("ID", false),
        ("NAME", false),
        ("PROJECT", false),
        ("DEPLOYED", false),
        ("LOGS", false),
        ("ADDRESS", false),
    ]);
    for function in functions {
        table.row(vec![
            field(function, "id"),
            field(function, "name"),
            project(function),
            deployed(function),
            field(function, "logs"),
            field(function, "url"),
        ]);
    }
    Ok(table.render())
}

pub fn function_one(body: &Value) -> Result<String, CloudError> {
    let function = one(body, "function")?;
    let mut out = Detail(vec![
        ("ID", field(function, "id")),
        ("Name", field(function, "name")),
        ("Project", project(function)),
        ("Address", field(function, "url")),
        (
            "Deployed",
            if deployed(function) == "no" {
                "no".to_string()
            } else {
                format!(
                    "{} at {}",
                    deployed(function),
                    field(function, "deployedAt")
                )
            },
        ),
        ("Logs", field(function, "logs")),
        (
            "Cap",
            format!(
                "{} of overage a month (changed in the dashboard)",
                cents_field(function, "capCents")
            ),
        ),
        (
            "Limits",
            function.get("limits").map_or(NONE.to_string(), |limits| {
                format!(
                    "{} ms CPU, {} subrequests per call",
                    field(limits, "cpuMs"),
                    field(limits, "subRequests")
                )
            }),
        ),
        ("Created", field(function, "createdAt")),
        ("Updated", field(function, "updatedAt")),
    ])
    .render();
    if let Some(vars) = function
        .get("vars")
        .and_then(Value::as_object)
        .filter(|vars| !vars.is_empty())
    {
        out.push_str("\nVariables\n");
        for (name, value) in vars {
            out.push_str(&format!(
                "  {}={}\n",
                clean(name),
                clean(value.as_str().unwrap_or(""))
            ));
        }
    }
    if let Some(secrets) = function
        .get("secrets")
        .and_then(Value::as_array)
        .filter(|secrets| !secrets.is_empty())
    {
        out.push_str("\nSecrets (names only; values are not readable)\n");
        for name in secrets {
            out.push_str(&format!("  {}\n", clean(name.as_str().unwrap_or(""))));
        }
    }
    Ok(out)
}

pub fn regions(body: &Value) -> Result<String, CloudError> {
    let regions = items(body, "regions")?;
    let mut table = Table::new(&[
        ("ID", false),
        ("CITY", false),
        ("COUNTRY", false),
        ("CONTINENT", false),
    ]);
    for region in regions {
        table.row(vec![
            field(region, "id"),
            field(region, "city"),
            field(region, "country"),
            field(region, "continent"),
        ]);
    }
    Ok(table.render())
}

pub fn usage(body: &Value) -> Result<String, CloudError> {
    let graphs = items(body, "graphs")?;
    let mut out = Detail(vec![
        ("Month", field(body, "month")),
        ("To date", cents_field(body, "toDateCents")),
        ("Projected", cents_field(body, "projectedCents")),
        ("Worst case", cents_field(body, "worstCaseCents")),
    ])
    .render();
    if graphs.is_empty() {
        out.push_str("\nno active graphs\n");
        return Ok(out);
    }
    let mut table = Table::new(&[
        ("GRAPH", false),
        ("QUERIES", true),
        ("WRITES", true),
        ("STORAGE GB", true),
        ("EGRESS GB", true),
        ("NODES", true),
        ("TO DATE", true),
        ("PROJECTED", true),
        ("STATE", false),
    ]);
    for graph in graphs {
        let usage = graph.get("usage").unwrap_or(&Value::Null);
        let nodes = graph.get("nodes").unwrap_or(&Value::Null);
        let paused = graph.get("paused").and_then(Value::as_bool) == Some(true);
        table.row(vec![
            field(graph, "graph"),
            quantity(number(usage, "queries")),
            quantity(number(usage, "writes")),
            quantity(number(usage, "storageGb")),
            quantity(number(usage, "egressGb")),
            format!("{}/{}", field(nodes, "count"), field(nodes, "included")),
            graph
                .get("toDate")
                .map_or(NONE.to_string(), |bill| cents_field(bill, "totalCents")),
            graph
                .get("projected")
                .map_or(NONE.to_string(), |bill| cents_field(bill, "totalCents")),
            if paused {
                "paused at cap".to_string()
            } else {
                "ok".to_string()
            },
        ]);
    }
    out.push('\n');
    out.push_str(&table.render());
    Ok(out)
}

pub fn logs(body: &Value) -> Result<String, CloudError> {
    let entries = items(body, "entries")?;
    let mut out = format!(
        "{}: logs {}, {} to {} (kept {} days)\n",
        field(body, "function"),
        field(body, "logs"),
        field(body, "from"),
        field(body, "to"),
        field(body, "retentionDays")
    );
    if entries.is_empty() {
        out.push_str("no requests in this window\n");
    }
    for entry in entries {
        out.push_str(&format!(
            "{}  {} {}  {}  {} ms  {}\n",
            field(entry, "time"),
            field(entry, "method"),
            field(entry, "path"),
            field(entry, "status"),
            quantity(number(entry, "durationMs")),
            field(entry, "outcome"),
        ));
        if let Some(exception) = entry.get("exception").and_then(Value::as_str) {
            out.push_str(&format!("    ! {}\n", clean(exception)));
        }
        for line in entry
            .get("logs")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let message = line.get("message").and_then(Value::as_str).unwrap_or("");
            let mut parts = message.split('\n');
            out.push_str(&format!(
                "    {}  {}\n",
                field(line, "level"),
                clean(parts.next().unwrap_or(""))
            ));
            for more in parts {
                out.push_str(&format!("        {}\n", clean(more)));
            }
        }
        if entry.get("logsTruncated").and_then(Value::as_bool) == Some(true) {
            out.push_str("    (more console lines were written than are kept)\n");
        }
    }
    if let Some(cursor) = body.get("cursor").and_then(Value::as_str) {
        out.push_str(&format!(
            "more: pass --cursor {} for the next page\n",
            clean(cursor)
        ));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn table_aligns_columns_and_right_aligns_numbers() {
        let mut table = Table::new(&[("ID", false), ("N", true), ("LAST", false)]);
        table.row(vec!["a".into(), "5".into(), "x".into()]);
        table.row(vec!["longer".into(), "1234".into(), "".into()]);
        let expected = ["ID         N  LAST", "a          5  x", "longer  1234"].join("\n") + "\n";
        assert_eq!(table.render(), expected);
    }

    #[test]
    fn control_characters_never_reach_the_terminal() {
        let body = json!({"projects":[{"id":"p1","name":"evil\u{1b}[31m\nname","createdAt":"t","graphs":[]}]});
        let out = projects(&body).unwrap();
        assert!(!out.contains('\u{1b}'), "{out:?}");
        assert_eq!(
            out.lines().count(),
            2,
            "a newline in a name must not add a row: {out:?}"
        );
    }

    #[test]
    fn money_and_sizes() {
        assert_eq!(cents(Some(1234)), "$12.34");
        assert_eq!(cents(Some(5)), "$0.05");
        assert_eq!(cents(Some(-250)), "-$2.50");
        assert_eq!(bytes(999), "999 B");
        assert_eq!(bytes(1_500_000), "1.5 MB");
    }

    #[test]
    fn a_body_that_is_not_the_apis_is_an_error_not_an_empty_list() {
        assert!(projects(&json!({"hello":"world"})).is_err());
    }
}
