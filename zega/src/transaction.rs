//! APS 54 phase 2 and APS 52: several steps, committed together or not at
//! all. Graph steps (ZQL) work on both stores; SQL steps need
//! [`crate::Storage::Sqlite`], where the application's SQL changes and the
//! graph's one `zega_log` entry commit in the same SQLite transaction.
//!
//! A transaction holds the graph lock (and, on SQLite, the log's connection)
//! from start to commit: one writer. Calling [`Zega::sql`] or another write
//! on the same `Zega` from inside the closure would wait for itself; use the
//! [`Transaction`] given to the closure instead.

use std::sync::MutexGuard;

use serde_json::Value as Json;

use crate::graph::Graph;
use crate::journal::{Pending, Target};
use crate::{Result, Zega, ZegaError};

/// The steps of one [`Zega::transaction`].
pub struct Transaction<'z> {
    zega: &'z Zega,
    graph: MutexGuard<'z, Graph>,
    pending: Option<Pending>,
    #[cfg(all(feature = "sqlite", not(target_arch = "wasm32")))]
    log: Option<crate::wal::sqlite::LogTx<'z>>,
    /// A step failed: the transaction can only be rolled back.
    failed: bool,
}

impl Zega {
    /// Run `f`'s steps as one transaction: if `f` returns `Err`, any step
    /// fails, or the commit fails, nothing it did is kept, in memory or on
    /// disk. Later steps see earlier ones.
    pub fn transaction<T>(&self, f: impl FnOnce(&mut Transaction<'_>) -> Result<T>) -> Result<T> {
        self.run_transaction(false, f)
    }

    /// A transaction for schema changes SQLite cannot make with ALTER TABLE:
    /// rebuilding a table (create the new one, `INSERT … SELECT`, drop the old,
    /// rename) without its children's ON DELETE CASCADE firing as the old
    /// parent is dropped. Foreign keys are off while it runs; before it
    /// commits every foreign key of every table is checked, and a row that
    /// names no parent fails it with a `constraint` error naming the table,
    /// its columns, the parent and how many rows, keeping nothing. Without
    /// [`crate::Storage::Sqlite`] it is [`Zega::transaction`].
    pub fn migration<T>(&self, f: impl FnOnce(&mut Transaction<'_>) -> Result<T>) -> Result<T> {
        self.run_transaction(true, f)
    }

    #[cfg_attr(not(all(feature = "sqlite", not(target_arch = "wasm32"))), allow(unused_variables))]
    fn run_transaction<T>(&self, migration: bool, f: impl FnOnce(&mut Transaction<'_>) -> Result<T>) -> Result<T> {
        let graph = self.lock_graph()?;
        #[cfg(all(feature = "sqlite", not(target_arch = "wasm32")))]
        let log = match self.wal.sqlite_log() {
            Some(log) if !self.in_memory => Some(if migration { log.begin_migration()? } else { log.begin()? }),
            _ => None,
        };
        let mut tx = Transaction {
            zega: self,
            graph,
            pending: Some(Pending::default()),
            #[cfg(all(feature = "sqlite", not(target_arch = "wasm32")))]
            log,
            failed: false,
        };
        let value = f(&mut tx)?;
        tx.commit()?;
        Ok(value)
    }
}

impl Transaction<'_> {
    /// A ZQL statement against `schema` (as [`Zega::run_lang`]).
    pub fn zql(&mut self, schema: &str, zql: &str) -> Result<Json> {
        let zega = self.zega;
        let ran = zega.run_lang_held(&mut self.graph, self.pending.as_mut().expect("open"), schema, zql);
        self.step(ran)
    }

    /// A whole `.zql` document: schema, then every block (as [`Zega::apply_zql`]).
    pub fn document(&mut self, source: &str) -> Result<Json> {
        let zega = self.zega;
        let ran = zega.apply_zql_held(&mut self.graph, self.pending.as_mut().expect("open"), source);
        self.step(ran)
    }

    /// Store `source` as the graph's schema (as [`Zega::set_schema`]): later
    /// steps run against it, and it is kept only if the transaction commits.
    pub fn schema(&mut self, source: &str) -> Result<crate::SchemaDiffReport> {
        let zega = self.zega;
        let stored = zega.set_schema_held(&mut self.graph, self.pending.as_mut().expect("open"), source);
        self.step(stored)
    }

    /// Delete what `flush` names (see [`crate::receipts::Flush`]): its
    /// relationships, then its nodes with every relationship they still have.
    pub fn flush(&mut self, flush: &crate::receipts::Flush) -> Result<()> {
        let pending = self.pending.as_mut().expect("open");
        let flushed = crate::journal::atomically_into(&mut self.graph, Target::Pending(pending), |graph, journal| {
            let rels: Vec<_> = graph
                .relationships()
                .filter(|rel| flush.all || flush.relationships.iter().any(|kind| kind == rel.kind))
                .map(|rel| rel.id)
                .collect();
            for id in rels {
                journal.delete_relationship(graph, id);
            }
            let nodes: Vec<_> = graph
                .nodes()
                .filter(|node| flush.all || flush.labels.iter().any(|label| node.has_label(label)))
                .map(|node| node.id)
                .collect();
            for id in nodes {
                journal.delete_node(graph, id);
            }
            Ok::<_, ZegaError>(())
        });
        self.step(flushed)
    }

    /// The receipt of `(source_id, txid)`, if it was delivered.
    pub(crate) fn receipt(&self, source_id: &str, txid: &str) -> Option<crate::receipts::Receipt> {
        self.graph.receipts.get(&(source_id.to_string(), txid.to_string())).cloned()
    }

    /// Keep `receipt` with this transaction's writes.
    pub(crate) fn record_receipt(&mut self, receipt: crate::receipts::Receipt) {
        self.pending.as_mut().expect("open").record_receipt(&mut self.graph, receipt);
    }

    /// Drop every receipt of `source_id` but `keep`'s, with this transaction's writes.
    pub(crate) fn forget_receipts(&mut self, source_id: &str, keep: &str) {
        let keys: Vec<_> = self.graph.receipts.keys().filter(|(source, txid)| source == source_id && txid != keep).cloned().collect();
        let pending = self.pending.as_mut().expect("open");
        for key in keys {
            pending.forget_receipt(&mut self.graph, key);
        }
    }

    fn step<T>(&mut self, result: Result<T>) -> Result<T> {
        if self.failed {
            return Err(ZegaError::Execution("an earlier step of this transaction failed".into()));
        }
        if result.is_err() {
            self.failed = true;
        }
        result
    }

    fn commit(mut self) -> Result<()> {
        if self.failed {
            return Err(ZegaError::Execution("an earlier step of this transaction failed".into()));
        }
        let pending = self.pending.take().expect("open");
        let zega = self.zega;
        #[cfg(all(feature = "sqlite", not(target_arch = "wasm32")))]
        let log = self.log.take();
        pending.commit(&mut self.graph, |ops| {
            #[cfg(all(feature = "sqlite", not(target_arch = "wasm32")))]
            if let Some(log) = log {
                return log.commit(crate::wal::entry(ops).as_ref()).map_err(sql::to_error);
            }
            zega.wal.append_statement(ops).map_err(ZegaError::from)
        })
    }
}

impl Drop for Transaction<'_> {
    fn drop(&mut self) {
        // Not committed (an error, an early return or a panic): take back the
        // graph's writes; dropping the log's transaction rolls SQLite back.
        if let Some(pending) = self.pending.take() {
            pending.abandon(&mut self.graph);
        }
    }
}

#[cfg(all(feature = "sqlite", not(target_arch = "wasm32")))]
pub use sql::{SqlResult, SqlValue};

#[cfg(all(feature = "sqlite", not(target_arch = "wasm32")))]
mod sql {
    use super::Transaction;
    use crate::wal::sqlite::{Limits, SqlFailure};
    use crate::{Result, Zega, ZegaError};
    use turso_core::Value;
    use turso_parser::ast::{Cmd, Stmt};
    use turso_parser::token::TokenType;

    /// The most rows, and bytes of values, one statement may return.
    const MAX_ROWS: usize = 100_000;
    const MAX_BYTES: usize = 64 << 20;

    /// A SQL value, as SQLite stores it.
    #[derive(Clone, Debug, PartialEq)]
    pub enum SqlValue {
        Null,
        Integer(i64),
        Real(f64),
        Text(String),
        Blob(Vec<u8>),
    }

    /// One statement's result.
    #[derive(Clone, Debug, PartialEq)]
    pub struct SqlResult {
        pub columns: Vec<String>,
        pub rows: Vec<Vec<SqlValue>>,
        /// Rows inserted, updated or deleted (0 for any other statement).
        pub changes: u64,
    }

    fn error(code: &'static str, message: impl Into<String>) -> ZegaError {
        ZegaError::Sql { code, message: message.into() }
    }

    pub(super) fn to_error(failure: SqlFailure) -> ZegaError {
        match failure {
            SqlFailure::Constraint(message) => error("constraint", message),
            SqlFailure::Statement(message) => error("sql_error", message),
            SqlFailure::TooLarge(message) => error("too_large", message),
            SqlFailure::Interrupted => error("sql_error", "interrupted"),
            SqlFailure::Durability(wal) => ZegaError::Wal(wal),
        }
    }

    /// Exactly one statement, of a kind applications may run, touching no
    /// `zega_*` table except to read it. Returns whether it changes rows.
    fn check(sql: &str) -> Result<bool> {
        let mut commands = turso_parser::parser::Parser::new(sql.as_bytes());
        let first = match commands.next() {
            Some(Ok(command)) => command,
            Some(Err(parse)) => return Err(error("sql_error", parse.to_string())),
            None => return Err(error("sql_error", "no SQL statement")),
        };
        if commands.next().is_some() {
            return Err(error("sql_error", "one SQL statement at a time; use a transaction for several"));
        }
        let Cmd::Stmt(statement) = first else {
            return Err(error("sql_error", "EXPLAIN is not available"));
        };
        let (changes_rows, read_only) = match statement {
            Stmt::Select(_) => (false, true),
            Stmt::Insert { .. } | Stmt::Update(_) | Stmt::Delete { .. } => (true, false),
            Stmt::CreateTable { .. } | Stmt::CreateIndex { .. } | Stmt::DropTable { .. } | Stmt::DropIndex { .. } | Stmt::AlterTable(_) => (false, false),
            _ => {
                return Err(error(
                    "sql_error",
                    "only SELECT, INSERT, UPDATE, DELETE, CREATE/DROP TABLE and INDEX, and ALTER TABLE are available",
                ))
            }
        };
        if !read_only && names_a_reserved_table(sql)? {
            return Err(error("reserved_table", "zega_* tables belong to Zega: only SELECT may read them"));
        }
        Ok(changes_rows)
    }

    /// Whether any identifier, quoted or not, anywhere in `sql` starts with
    /// `zega_`: tables in subqueries, CTEs and qualified names included.
    fn names_a_reserved_table(sql: &str) -> Result<bool> {
        for token in turso_parser::lexer::Lexer::new(sql.as_bytes()) {
            let token = token.map_err(|e| error("sql_error", e.to_string()))?;
            if token.token_type == TokenType::TK_ID {
                let name = String::from_utf8_lossy(token.value);
                let name = name.trim_matches(|c| matches!(c, '"' | '`' | '[' | ']'));
                if name.to_ascii_lowercase().starts_with("zega_") {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    fn to_turso(value: &SqlValue) -> Value {
        match value {
            SqlValue::Null => Value::Null,
            SqlValue::Integer(i) => Value::from_i64(*i),
            SqlValue::Real(f) => Value::from_f64(*f),
            SqlValue::Text(text) => Value::build_text(text.clone()),
            SqlValue::Blob(bytes) => Value::from_blob(bytes.clone()),
        }
    }

    fn from_turso(value: Value) -> SqlValue {
        match value {
            Value::Null => SqlValue::Null,
            Value::Numeric(turso_core::Numeric::Integer(i)) => SqlValue::Integer(i),
            Value::Numeric(turso_core::Numeric::Float(f)) => SqlValue::Real(f64::from(f)),
            Value::Text(text) => SqlValue::Text(text.as_str().to_string()),
            Value::Blob(bytes) => SqlValue::Blob(bytes.to_vec()),
        }
    }

    impl Zega {
        /// One SQL statement with parameters `?1`, `?2`, …, in its own
        /// transaction. Needs a database opened with [`crate::Storage::Sqlite`].
        pub fn sql(&self, sql: &str, params: &[SqlValue]) -> Result<SqlResult> {
            self.transaction(|tx| tx.sql(sql, params))
        }
    }

    impl Transaction<'_> {
        /// One SQL statement inside the transaction.
        pub fn sql(&mut self, sql: &str, params: &[SqlValue]) -> Result<SqlResult> {
            let ran = self.run_sql(sql, params);
            self.step(ran)
        }

        fn run_sql(&mut self, sql: &str, params: &[SqlValue]) -> Result<SqlResult> {
            let Some(log) = self.log.as_mut() else {
                return Err(error("sql_unavailable", "SQL needs a database opened with storage sqlite"));
            };
            let counts_changes = check(sql)?;
            let limit = self.zega.query_time_limit;
            let limits = Limits {
                deadline: limit.map(|limit| std::time::Instant::now() + limit),
                max_rows: MAX_ROWS,
                max_bytes: MAX_BYTES,
                counts_changes,
            };
            let params: Vec<Value> = params.iter().map(to_turso).collect();
            let rows = log.execute(sql, &params, &limits).map_err(|failure| match (failure, limit) {
                (SqlFailure::Interrupted, Some(limit)) => ZegaError::QueryTimeLimit { limit },
                (failure, _) => to_error(failure),
            })?;
            Ok(SqlResult {
                columns: rows.columns,
                rows: rows.rows.into_iter().map(|row| row.into_iter().map(from_turso).collect()).collect(),
                changes: rows.changes,
            })
        }
    }
}
