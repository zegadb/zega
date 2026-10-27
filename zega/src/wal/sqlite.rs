//! APS 54 phase 1: the log kept as rows of a SQLite database, written by
//! Turso's Rust engine (`turso_core`), instead of in `wal.bin`.
//!
//! Each entry is one row of `zega_log`: the same bincode bytes and CRC the
//! file log writes, committed as one SQLite transaction with
//! `synchronous = FULL`. A row is in the log once its commit returns, so
//! there is no torn tail to repair and no rotation file to settle.
//!
//! Positions are the bytes the file log would hold for the same entries
//! (`pos` is where an entry would start). [`Wal::end`](super::Wal) and
//! [`Wal::rotate`](super::Wal) mean the same on both stores, so checkpoint
//! sizing (`checkpoint::Checkpointer::due`) is shared, not re-derived.
//!
//! [`SqliteLog::rotate`] is one transaction: delete the entries before the
//! checkpoint, shift the rest, insert the checkpoint's `ReplaceGraph` head.
//! A crash leaves either the old log or the new one.

use std::num::NonZero;
use std::path::Path;
use std::sync::{Arc, Mutex};

use turso_core::{Connection, Database, PlatformIO, SqliteDialect, Value, IO};

use super::{decode_exact, Operation, WalError, ENTRY_HEADER_LEN, WAL_FILE_HEADER_LEN};

/// The file a SQLite-backed store keeps its log in, beside `graphs/`.
pub const SQLITE_LOG_FILE: &str = "zega.db";

/// Bumped when `zega_log`'s columns or meaning change.
const FORMAT: i64 = 1;

const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS zega_meta (key TEXT PRIMARY KEY, value INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS zega_log (seq INTEGER PRIMARY KEY, pos INTEGER NOT NULL, crc INTEGER NOT NULL, ops BLOB NOT NULL);";

pub(crate) struct SqliteLog {
    state: Mutex<State>,
}

struct State {
    conn: Arc<Connection>,
    // Held so the database outlives every statement on `conn`.
    _db: Arc<Database>,
    next_seq: i64,
    /// Where the next entry starts, in file-log bytes.
    end: u64,
    /// Set by a commit whose outcome is unknown; every later call returns it
    /// until the store is reopened, as the file log does after a failed sync.
    durability_error: Option<String>,
}

fn sql_error(context: &str, error: turso_core::LimboError) -> WalError {
    WalError::Io(std::io::Error::other(format!("{context}: {error}")))
}

fn entry_len(payload: &[u8]) -> u64 {
    ENTRY_HEADER_LEN + payload.len() as u64
}

fn param(index: usize) -> NonZero<usize> {
    NonZero::new(index).expect("SQL parameters are numbered from 1")
}

fn as_integer(value: &Value) -> Option<i64> {
    match value {
        Value::Numeric(turso_core::Numeric::Integer(i)) => Some(*i),
        _ => None,
    }
}

fn integer(value: &Value, column: &str) -> Result<i64, WalError> {
    as_integer(value).ok_or_else(|| WalError::Corruption {
        offset: 0,
        reason: format!("zega_log.{column} is not an integer"),
    })
}

impl SqliteLog {
    pub(crate) fn open(path: &Path) -> Result<Self, WalError> {
        let path_str = path
            .to_str()
            .ok_or_else(|| WalError::Io(std::io::Error::other("the data path is not valid UTF-8")))?;
        let io: Arc<dyn IO> = Arc::new(PlatformIO::new().map_err(|e| sql_error("starting SQLite I/O", e))?);
        let db = Database::open_file(io, path_str, Arc::new(SqliteDialect))
            .map_err(|e| sql_error("opening the SQLite log", e))?;
        let conn = db.connect().map_err(|e| sql_error("connecting to the SQLite log", e))?;
        conn.execute("PRAGMA journal_mode = wal")
            .and_then(|()| conn.execute("PRAGMA synchronous = FULL"))
            .and_then(|()| conn.execute(SCHEMA))
            .map_err(|e| sql_error("preparing the SQLite log", e))?;
        let format = query(&conn, "SELECT value FROM zega_meta WHERE key = 'format'", &[])?;
        match format.first().and_then(|row| row.first()) {
            None => {
                run(&conn, "INSERT INTO zega_meta (key, value) VALUES ('format', ?1)", &[Value::from_i64(FORMAT)])?;
            }
            Some(value) if integer(value, "format")? == FORMAT => {}
            Some(value) => {
                return Err(WalError::Corruption {
                    offset: 0,
                    reason: format!(
                        "zega_log format {} is not one this zega reads (it reads {FORMAT})",
                        integer(value, "format")?
                    ),
                })
            }
        }
        let last = query(&conn, "SELECT seq, pos, length(ops) FROM zega_log ORDER BY seq DESC LIMIT 1", &[])?;
        let (next_seq, end) = match last.first() {
            None => (1, WAL_FILE_HEADER_LEN),
            Some(row) => {
                let seq = integer(&row[0], "seq")?;
                let pos = integer(&row[1], "pos")? as u64;
                let len = integer(&row[2], "ops")? as u64;
                (seq + 1, pos + ENTRY_HEADER_LEN + len)
            }
        };
        Ok(SqliteLog {
            state: Mutex::new(State { conn, _db: db, next_seq, end, durability_error: None }),
        })
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, State>, WalError> {
        let state = self
            .state
            .lock()
            .map_err(|_| WalError::Durability("SQLite log lock poisoned".to_string()))?;
        if let Some(error) = &state.durability_error {
            return Err(WalError::Durability(error.clone()));
        }
        Ok(state)
    }

    /// One entry, durable when this returns.
    pub(crate) fn append(&self, payload: &[u8], crc: u32) -> Result<(), WalError> {
        let mut state = self.lock()?;
        let (seq, pos) = (state.next_seq, state.end);
        let insert = run(
            &state.conn,
            "INSERT INTO zega_log (seq, pos, crc, ops) VALUES (?1, ?2, ?3, ?4)",
            &[
                Value::from_i64(seq),
                Value::from_i64(pos as i64),
                Value::from_i64(i64::from(crc)),
                Value::from_blob(payload.to_vec()),
            ],
        );
        if let Err(error) = insert {
            // An autocommit insert that failed may or may not have reached
            // the disk: refuse writes until a reopen reads which.
            let message = format!("SQLite log append failed ({error}); the log refuses writes until the store is reopened");
            state.durability_error = Some(message.clone());
            return Err(WalError::Durability(message));
        }
        state.next_seq += 1;
        state.end += entry_len(payload);
        Ok(())
    }

    pub(crate) fn end(&self) -> Result<u64, WalError> {
        Ok(self.lock()?.end)
    }

    /// Every entry, in order, each checked against its CRC.
    pub(crate) fn iter(&self) -> Result<Vec<Operation>, WalError> {
        let state = self.lock()?;
        let rows = query(&state.conn, "SELECT pos, crc, ops FROM zega_log ORDER BY seq", &[])?;
        rows.iter()
            .map(|row| {
                let pos = integer(&row[0], "pos")? as u64;
                let expected = integer(&row[1], "crc")?;
                let Some(payload) = row[2].to_blob() else {
                    return Err(WalError::Corruption { offset: pos, reason: "zega_log.ops is not a blob".to_string() });
                };
                let actual = crc32fast::hash(payload);
                if i64::from(actual) != expected {
                    return Err(WalError::Corruption {
                        offset: pos,
                        reason: format!("checksum mismatch (expected {expected:#010x}, got {actual:#010x})"),
                    });
                }
                decode_exact(payload).map_err(|error| WalError::Corruption {
                    offset: pos,
                    reason: format!("invalid operation payload: {error}"),
                })
            })
            .collect()
    }

    /// Keep the entries from `from` on, behind `head`, in one transaction.
    /// Returns the new end.
    pub(crate) fn rotate(&self, from: u64, head: &[u8]) -> Result<u64, WalError> {
        let mut state = self.lock()?;
        if from < WAL_FILE_HEADER_LEN || from > state.end {
            return Err(WalError::Durability(format!(
                "checkpoint position {from} is outside the log (length {})",
                state.end
            )));
        }
        let head_end = WAL_FILE_HEADER_LEN + entry_len(head);
        let shift = head_end as i64 - from as i64;
        let conn = Arc::clone(&state.conn);
        let staged = (|| -> Result<(), WalError> {
            run(&conn, "BEGIN IMMEDIATE", &[])?;
            run(&conn, "DELETE FROM zega_log WHERE pos < ?1", &[Value::from_i64(from as i64)])?;
            // The head goes just before the first kept entry; that seq was
            // an entry before `from`, deleted above, or is the next one.
            let first = query(&conn, "SELECT min(seq) FROM zega_log", &[])?;
            let head_seq = match first.first().and_then(|row| row.first()).and_then(as_integer) {
                Some(seq) => seq - 1,
                None => state.next_seq,
            };
            run(&conn, "UPDATE zega_log SET pos = pos + ?1", &[Value::from_i64(shift)])?;
            run(
                &conn,
                "INSERT INTO zega_log (seq, pos, crc, ops) VALUES (?1, ?2, ?3, ?4)",
                &[
                    Value::from_i64(head_seq),
                    Value::from_i64(WAL_FILE_HEADER_LEN as i64),
                    Value::from_i64(i64::from(crc32fast::hash(head))),
                    Value::from_blob(head.to_vec()),
                ],
            )?;
            if head_seq >= state.next_seq {
                state.next_seq = head_seq + 1;
            }
            Ok(())
        })();
        if let Err(error) = staged {
            // Nothing is committed yet: the log is as it was.
            let _ = conn.execute("ROLLBACK");
            return Err(error);
        }
        if let Err(error) = conn.execute("COMMIT") {
            let message = format!(
                "SQLite log rotation could not commit ({error}); the log refuses writes until the store is reopened"
            );
            state.durability_error = Some(message.clone());
            return Err(WalError::Durability(message));
        }
        state.end = (state.end as i64 + shift) as u64;
        Ok(state.end)
    }
}

fn bind(statement: &mut turso_core::Statement, params: &[Value]) -> Result<(), WalError> {
    for (index, value) in params.iter().enumerate() {
        statement
            .bind_at(param(index + 1), value.clone())
            .map_err(|e| sql_error("binding a SQLite log parameter", e))?;
    }
    Ok(())
}

fn run(conn: &Arc<Connection>, sql: &str, params: &[Value]) -> Result<(), WalError> {
    let mut statement = conn.prepare(sql).map_err(|e| sql_error("preparing a SQLite log statement", e))?;
    bind(&mut statement, params)?;
    statement.run_ignore_rows().map_err(|e| sql_error("writing the SQLite log", e))
}

fn query(conn: &Arc<Connection>, sql: &str, params: &[Value]) -> Result<Vec<Vec<Value>>, WalError> {
    let mut statement = conn.prepare(sql).map_err(|e| sql_error("preparing a SQLite log query", e))?;
    bind(&mut statement, params)?;
    statement.run_collect_rows().map_err(|e| sql_error("reading the SQLite log", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn op(name: &str) -> (Vec<u8>, u32) {
        let op = Operation::DeleteNode { id: name.len() as u64 };
        let bytes = bincode::serialize(&op).unwrap();
        let crc = crc32fast::hash(&bytes);
        (bytes, crc)
    }

    fn ids(log: &SqliteLog) -> Vec<u64> {
        log.iter()
            .unwrap()
            .into_iter()
            .map(|op| match op {
                Operation::DeleteNode { id } => id,
                Operation::ReplaceGraph { .. } => 0,
                other => panic!("unexpected {other:?}"),
            })
            .collect()
    }

    #[test]
    fn reopen_continues_where_the_log_ended() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(SQLITE_LOG_FILE);
        let end = {
            let log = SqliteLog::open(&path).unwrap();
            for name in ["a", "bb", "ccc"] {
                let (bytes, crc) = op(name);
                log.append(&bytes, crc).unwrap();
            }
            log.end().unwrap()
        };
        let (one, _) = op("a");
        assert_eq!(end, WAL_FILE_HEADER_LEN + 3 * entry_len(&one), "positions are file-log bytes");
        let log = SqliteLog::open(&path).unwrap();
        assert_eq!(log.end().unwrap(), end);
        let (bytes, crc) = op("dddd");
        log.append(&bytes, crc).unwrap();
        assert_eq!(ids(&log), [1, 2, 3, 4]);
    }

    #[test]
    fn rotate_keeps_the_entries_after_the_checkpoint_behind_its_head() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(SQLITE_LOG_FILE);
        let log = SqliteLog::open(&path).unwrap();
        let (first, crc) = op("a");
        log.append(&first, crc).unwrap();
        let from = log.end().unwrap();
        for name in ["bb", "ccc"] {
            let (bytes, crc) = op(name);
            log.append(&bytes, crc).unwrap();
        }
        let head = bincode::serialize(&Operation::ReplaceGraph { file: "graphs/x.graph".into() }).unwrap();
        let end = log.rotate(from, &head).unwrap();
        assert_eq!(end, WAL_FILE_HEADER_LEN + entry_len(&head) + 2 * entry_len(&first));
        assert_eq!(ids(&log), [0, 2, 3]);
        drop(log);
        let log = SqliteLog::open(&path).unwrap();
        assert_eq!((log.end().unwrap(), ids(&log)), (end, vec![0, 2, 3]));
        // Rotating everything away leaves the head alone; appends follow it.
        let end = log.rotate(end, &head).unwrap();
        assert_eq!(end, WAL_FILE_HEADER_LEN + entry_len(&head));
        log.append(&first, crc).unwrap();
        assert_eq!(ids(&log), [0, 1]);
    }

    #[test]
    fn a_rotation_from_outside_the_log_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let log = SqliteLog::open(&dir.path().join(SQLITE_LOG_FILE)).unwrap();
        let (bytes, crc) = op("a");
        log.append(&bytes, crc).unwrap();
        let end = log.end().unwrap();
        assert!(log.rotate(end + 1, &bytes).is_err());
        assert!(log.rotate(0, &bytes).is_err());
        assert_eq!((log.end().unwrap(), ids(&log)), (end, vec![1]));
        log.append(&bytes, crc).unwrap();
    }

    #[test]
    fn a_damaged_entry_is_corruption() {
        let dir = tempfile::tempdir().unwrap();
        let log = SqliteLog::open(&dir.path().join(SQLITE_LOG_FILE)).unwrap();
        let (bytes, crc) = op("a");
        log.append(&bytes, crc).unwrap();
        log.append(&bytes, crc).unwrap();
        {
            let state = log.state.lock().unwrap();
            run(&state.conn, "UPDATE zega_log SET ops = ?1 WHERE seq = 2", &[Value::from_blob(vec![1, 2, 3])]).unwrap();
        }
        match log.iter() {
            Err(WalError::Corruption { offset, reason }) => {
                assert_eq!(offset, WAL_FILE_HEADER_LEN + entry_len(&bytes));
                assert!(reason.contains("checksum mismatch"), "{reason}");
            }
            other => panic!("expected corruption, got {other:?}"),
        }
    }

    #[test]
    fn a_log_of_another_format_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(SQLITE_LOG_FILE);
        {
            let log = SqliteLog::open(&path).unwrap();
            let state = log.state.lock().unwrap();
            run(&state.conn, "UPDATE zega_meta SET value = 99 WHERE key = 'format'", &[]).unwrap();
        }
        let refused = SqliteLog::open(&path).err().expect("opened format 99");
        assert!(refused.to_string().contains("format 99"), "{refused}");
    }
}
