//! APS 54 phase 2 and APS 52: `Zega::transaction` and `Zega::sql`. Graph-only
//! transactions run on both stores; SQL needs `Storage::Sqlite`, where an
//! application's SQL rows and the graph's writes commit together or not at all.

#![cfg(feature = "sqlite")]

use serde_json::Value as Json;
use zega::{SqlValue, Storage, Zega, ZegaError};

const P: &str = r#"schema { type P { name: String } } unique { P { name } }"#;

fn open(path: &str, storage: Storage) -> Zega {
    Zega::open(path).storage(storage).wal_flush_every_write().build().unwrap()
}

fn names(zega: &Zega) -> Vec<String> {
    let rows = match zega.run_lang(P, "query { P { name } }").unwrap() {
        Json::Array(rows) => rows,
        Json::Null => Vec::new(),
        row => vec![row],
    };
    let mut names: Vec<String> = rows.iter().map(|row| row["name"].as_str().unwrap().to_string()).collect();
    names.sort();
    names
}

fn code(error: ZegaError) -> &'static str {
    match error {
        ZegaError::Sql { code, .. } => code,
        other => panic!("expected a SQL error, got {other:?}"),
    }
}

fn text(s: &str) -> SqlValue {
    SqlValue::Text(s.into())
}

/// Two graph steps commit together; a failing step, or an `Err` from the
/// closure, keeps neither, in memory and after a reopen.
fn graph_steps_are_all_or_nothing(storage: Storage) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_str().unwrap();
    {
        let zega = open(path, storage);
        zega.transaction(|tx| {
            tx.zql(P, r#"mutation { P(name: "a") }"#)?;
            // A later step sees the earlier one: "a" is now a duplicate.
            assert!(tx.zql(P, r#"mutation { P(name: "a") }"#).is_err());
            Ok(())
        })
        .expect_err("a failed step fails the transaction");
        assert_eq!(names(&zega), Vec::<String>::new());
        zega.transaction(|tx| {
            tx.zql(P, r#"mutation { P(name: "b") }"#)?;
            Err::<(), _>(ZegaError::Execution("changed my mind".into()))
        })
        .unwrap_err();
        assert_eq!(names(&zega), Vec::<String>::new());
        zega.transaction(|tx| {
            tx.zql(P, r#"mutation { P(name: "c") }"#)?;
            tx.document(&format!("{P}\nmutation {{ P(name: \"d\") }}\nmutation {{ P(name: \"e\") }}"))
        })
        .unwrap();
        assert_eq!(names(&zega), ["c", "d", "e"]);
    }
    assert_eq!(names(&open(path, storage)), ["c", "d", "e"]);
}

#[test]
fn file_graph_steps_are_all_or_nothing() {
    graph_steps_are_all_or_nothing(Storage::File);
}

#[test]
fn sqlite_graph_steps_are_all_or_nothing() {
    graph_steps_are_all_or_nothing(Storage::Sqlite);
}

#[test]
fn sql_needs_sqlite_storage() {
    let dir = tempfile::tempdir().unwrap();
    let file = open(dir.path().to_str().unwrap(), Storage::File);
    assert_eq!(code(file.sql("SELECT 1", &[]).unwrap_err()), "sql_unavailable");
    let memory = Zega::in_memory().build().unwrap();
    assert_eq!(code(memory.sql("SELECT 1", &[]).unwrap_err()), "sql_unavailable");
}

fn orders(zega: &Zega) -> Vec<SqlValue> {
    zega.sql("SELECT id FROM orders ORDER BY id", &[]).unwrap().rows.into_iter().map(|mut row| row.remove(0)).collect()
}

#[test]
fn sql_and_graph_commit_together_or_not_at_all() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_str().unwrap();
    {
        let zega = open(path, Storage::Sqlite);
        let created = zega.sql("CREATE TABLE orders (id INTEGER PRIMARY KEY, who TEXT NOT NULL)", &[]).unwrap();
        assert_eq!(created.changes, 0);
        zega.transaction(|tx| {
            let inserted = tx.sql("INSERT INTO orders (id, who) VALUES (?1, ?2), (?3, ?4)", &[SqlValue::Integer(1), text("a"), SqlValue::Integer(2), text("a")])?;
            assert_eq!(inserted.changes, 2);
            tx.zql(P, r#"mutation { P(name: "a") }"#)?;
            Ok(())
        })
        .unwrap();
        // The graph step fails (a duplicate), so the SQL insert goes too.
        assert!(zega
            .transaction(|tx| {
                tx.sql("INSERT INTO orders (id, who) VALUES (3, 'a')", &[])?;
                tx.zql(P, r#"mutation { P(name: "a") }"#)
            })
            .is_err());
        // The SQL step fails (a duplicate key), so the graph write goes too.
        let refused = zega
            .transaction(|tx| {
                tx.zql(P, r#"mutation { P(name: "b") }"#)?;
                tx.sql("INSERT INTO orders (id, who) VALUES (1, 'b')", &[])
            })
            .unwrap_err();
        assert_eq!(code(refused), "constraint");
        assert_eq!(orders(&zega), [SqlValue::Integer(1), SqlValue::Integer(2)]);
        assert_eq!(names(&zega), ["a"]);
        let read = zega.sql("SELECT who, count(*) AS n FROM orders GROUP BY who", &[]).unwrap();
        assert_eq!(read.columns, ["who", "n"]);
        assert_eq!(read.rows, [vec![text("a"), SqlValue::Integer(2)]]);
        assert_eq!(read.changes, 0);
    }
    let zega = open(path, Storage::Sqlite);
    assert_eq!(orders(&zega), [SqlValue::Integer(1), SqlValue::Integer(2)]);
    assert_eq!(names(&zega), ["a"]);
}

#[test]
fn zega_tables_can_be_read_but_never_written() {
    let dir = tempfile::tempdir().unwrap();
    let zega = open(dir.path().to_str().unwrap(), Storage::Sqlite);
    zega.sql("CREATE TABLE t (x INTEGER)", &[]).unwrap();
    for sql in [
        "DELETE FROM zega_log",
        "UPDATE zega_log SET ops = x'00'",
        "INSERT INTO zega_meta (key, value) VALUES ('format', 9)",
        "DROP TABLE zega_log",
        "ALTER TABLE zega_log ADD COLUMN y INTEGER",
        "ALTER TABLE t RENAME TO zega_t",
        "CREATE TABLE zega_mine (x INTEGER)",
        "CREATE INDEX i ON zega_log (pos)",
        "DELETE FROM main.zega_log",
        r#"DELETE FROM "ZEGA_LOG""#,
        "DELETE FROM [zega_log]",
        "WITH gone AS (SELECT seq FROM zega_log) DELETE FROM t WHERE x IN (SELECT seq FROM gone)",
        "INSERT INTO t SELECT seq FROM zega_log",
        "CREATE TABLE copy AS SELECT * FROM zega_log",
    ] {
        assert_eq!(code(zega.sql(sql, &[]).unwrap_err()), "reserved_table", "{sql}");
    }
    let seen = zega.sql("SELECT count(*) FROM zega_meta WHERE key = 'format'", &[]).unwrap();
    assert_eq!(seen.rows, [vec![SqlValue::Integer(1)]]);
    // Strings are values, not names.
    zega.sql("CREATE TABLE notes (body TEXT)", &[]).unwrap();
    zega.sql("INSERT INTO notes (body) VALUES ('zega_log is reserved')", &[]).unwrap();
}

#[test]
fn one_allowed_statement_at_a_time() {
    let dir = tempfile::tempdir().unwrap();
    let zega = open(dir.path().to_str().unwrap(), Storage::Sqlite);
    zega.sql("CREATE TABLE t (x INTEGER)", &[]).unwrap();
    for sql in [
        "INSERT INTO t VALUES (1); INSERT INTO t VALUES (2)",
        "BEGIN",
        "COMMIT",
        "SAVEPOINT s",
        "PRAGMA journal_mode = delete",
        "ATTACH 'other.db' AS other",
        "VACUUM",
        "CREATE TRIGGER g AFTER INSERT ON t BEGIN DELETE FROM t; END",
        "CREATE VIEW v AS SELECT * FROM t",
        "EXPLAIN SELECT 1",
        "",
        "SELEC 1",
    ] {
        assert_eq!(code(zega.sql(sql, &[]).unwrap_err()), "sql_error", "{sql:?}");
    }
    assert_eq!(zega.sql("SELECT count(*) FROM t", &[]).unwrap().rows, [vec![SqlValue::Integer(0)]]);
}

#[test]
fn a_result_too_large_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let zega = open(dir.path().to_str().unwrap(), Storage::Sqlite);
    let big = "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 100001) SELECT i FROM n";
    assert_eq!(code(zega.sql(big, &[]).unwrap_err()), "too_large");
    let fits = zega.sql(&big.replace("100001", "1000"), &[]).unwrap();
    assert_eq!(fits.rows.len(), 1000);
}

#[test]
fn checkpoints_wait_for_transactions() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_str().unwrap();
    {
        let zega = std::sync::Arc::new(open(path, Storage::Sqlite));
        zega.sql("CREATE TABLE seen (i INTEGER PRIMARY KEY)", &[]).unwrap();
        let checkpoints = {
            let zega = zega.clone();
            std::thread::spawn(move || {
                for _ in 0..20 {
                    zega.checkpoint().unwrap();
                }
            })
        };
        for i in 0..200i64 {
            zega.transaction(|tx| {
                tx.sql("INSERT INTO seen (i) VALUES (?1)", &[SqlValue::Integer(i)])?;
                tx.zql(P, &format!(r#"mutation {{ P(name: "n{i}") }}"#))
            })
            .unwrap();
        }
        checkpoints.join().unwrap();
    }
    let zega = open(path, Storage::Sqlite);
    assert_eq!(names(&zega).len(), 200);
    assert_eq!(zega.sql("SELECT count(*) FROM seen", &[]).unwrap().rows, [vec![SqlValue::Integer(200)]]);
}

const CHILD_DIR: &str = "ZEGA_TRANSACTION_CRASH_CHILD_DIR";

/// The child: commits `INSERT INTO seen (i)` with the node `n<i>` for
/// i = 1, 2, … and prints `i` once each commit returns, until it is killed.
#[test]
#[ignore = "run only as the child of killed_mid_transactions_keeps_rows_and_nodes_paired"]
fn transaction_crash_child() {
    let Ok(dir) = std::env::var(CHILD_DIR) else { return };
    let zega = Zega::open(&dir).storage(Storage::Sqlite).build().unwrap();
    zega.sql("CREATE TABLE IF NOT EXISTS seen (i INTEGER PRIMARY KEY)", &[]).unwrap();
    use std::io::Write;
    let mut out = std::io::stdout().lock();
    let mut i = 0i64;
    loop {
        i += 1;
        zega.transaction(|tx| {
            tx.sql("INSERT INTO seen (i) VALUES (?1)", &[SqlValue::Integer(i)])?;
            tx.zql(P, &format!(r#"mutation {{ P(name: "n{i}") }}"#))
        })
        .unwrap();
        writeln!(out, "ack {i}").unwrap();
        out.flush().unwrap();
    }
}

/// SIGKILL a process committing SQL + graph transactions: on reopen, every
/// acknowledged transaction is there, and the SQL rows and graph nodes are
/// the same set: never one without the other.
#[test]
fn killed_mid_transactions_keeps_rows_and_nodes_paired() {
    use std::io::BufRead;
    for kill_after in [1usize, 9, 50] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_str().unwrap();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "transaction_crash_child", "--ignored", "--nocapture", "--test-threads=1"])
            .env(CHILD_DIR, path)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let mut acked = 0;
        for line in std::io::BufReader::new(child.stdout.take().unwrap()).lines() {
            if let Some(i) = line.unwrap().strip_prefix("ack ") {
                acked = i.parse::<usize>().unwrap();
                if acked >= kill_after {
                    break;
                }
            }
        }
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(acked >= kill_after, "the child stopped after {acked}");
        let zega = open(path, Storage::Sqlite);
        let mut nodes: Vec<i64> = names(&zega).iter().map(|name| name[1..].parse().unwrap()).collect();
        nodes.sort_unstable();
        let rows: Vec<i64> = zega
            .sql("SELECT i FROM seen ORDER BY i", &[])
            .unwrap()
            .rows
            .into_iter()
            .map(|row| match row[0] {
                SqlValue::Integer(i) => i,
                ref other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(rows, nodes, "SQL rows and graph nodes differ after a kill");
        assert!(rows.len() >= acked, "{acked} acknowledged, {} survived", rows.len());
        assert_eq!(rows, (1..=rows.len() as i64).collect::<Vec<_>>());
    }
}
