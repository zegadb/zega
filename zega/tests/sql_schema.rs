//! zega#189: what an embedder's schema sync needs from `Storage::Sqlite`.
//! Foreign keys are enforced, `Zega::migration` rebuilds a parent table
//! without cascading into its children, and the catalog, defaults,
//! RETURNING, JSON functions and multi-step DDL work through `Zega::sql`.

#![cfg(feature = "sqlite")]

use zega::{SqlResult, SqlValue, Storage, Zega, ZegaError};

fn open(path: &str) -> Zega {
    Zega::open(path).storage(Storage::Sqlite).wal_flush_every_write().build().unwrap()
}

fn int(result: SqlResult) -> i64 {
    match result.rows[0][0] {
        SqlValue::Integer(i) => i,
        ref other => panic!("expected an integer, got {other:?}"),
    }
}

fn refusal(error: ZegaError) -> (&'static str, String) {
    match error {
        ZegaError::Sql { code, message } => (code, message),
        other => panic!("expected a SQL error, got {other:?}"),
    }
}

/// users ← posts (ON DELETE CASCADE), with two users and three posts.
fn users_and_posts(zega: &Zega) {
    zega.sql("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL)", &[]).unwrap();
    zega.sql(
        "CREATE TABLE posts (id INTEGER PRIMARY KEY, user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE, title TEXT)",
        &[],
    )
    .unwrap();
    zega.sql("INSERT INTO users (id, name) VALUES (1, 'ada'), (2, 'bob')", &[]).unwrap();
    zega.sql("INSERT INTO posts (user_id, title) VALUES (1, 'a'), (1, 'b'), (2, 'c')", &[]).unwrap();
}

fn posts(zega: &Zega) -> i64 {
    int(zega.sql("SELECT count(*) FROM posts", &[]).unwrap())
}

/// The users table rebuilt with a new NOT NULL column: what ALTER TABLE cannot do.
fn rebuild_users(tx: &mut zega::Transaction<'_>) -> zega::Result<()> {
    tx.sql("CREATE TABLE users_new (id INTEGER PRIMARY KEY, name TEXT NOT NULL, email TEXT NOT NULL DEFAULT '')", &[])?;
    tx.sql("INSERT INTO users_new (id, name) SELECT id, name FROM users", &[])?;
    tx.sql("DROP TABLE users", &[])?;
    tx.sql("ALTER TABLE users_new RENAME TO users", &[])?;
    Ok(())
}

#[test]
fn foreign_keys_are_enforced_and_cascade_after_a_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_str().unwrap();
    users_and_posts(&open(path));
    let zega = open(path);
    let (code, message) = refusal(zega.sql("INSERT INTO posts (user_id, title) VALUES (999, 'orphan')", &[]).unwrap_err());
    assert_eq!(code, "constraint", "{message}");
    zega.sql("DELETE FROM users WHERE id = 1", &[]).unwrap();
    assert_eq!(posts(&zega), 1, "deleting a user deletes its posts");
}

#[test]
fn a_plain_transaction_rebuild_cascades_into_the_children() {
    // Why `migration` exists: dropping the old parent deletes every child.
    let dir = tempfile::tempdir().unwrap();
    let zega = open(dir.path().to_str().unwrap());
    users_and_posts(&zega);
    zega.transaction(rebuild_users).unwrap();
    assert_eq!(posts(&zega), 0);
}

#[test]
fn a_migration_rebuilds_a_parent_and_keeps_its_children() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_str().unwrap();
    {
        let zega = open(path);
        users_and_posts(&zega);
        zega.migration(rebuild_users).unwrap();
        assert_eq!(posts(&zega), 3);
        let columns = zega.sql("SELECT name FROM pragma_table_info('users') ORDER BY cid", &[]).unwrap();
        assert_eq!(columns.rows.len(), 3, "the new email column is there");
        // Foreign keys are on again once it has committed.
        assert_eq!(refusal(zega.sql("INSERT INTO posts (user_id) VALUES (999)", &[]).unwrap_err()).0, "constraint");
        zega.sql("DELETE FROM users WHERE id = 2", &[]).unwrap();
        assert_eq!(posts(&zega), 2, "the rebuilt parent still cascades");
    }
    assert_eq!(posts(&open(path)), 2);
}

#[test]
fn a_migration_that_leaves_a_dangling_row_keeps_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let zega = open(dir.path().to_str().unwrap());
    users_and_posts(&zega);
    let error = zega
        .migration(|tx| {
            rebuild_users(tx)?;
            tx.sql("INSERT INTO posts (user_id, title) VALUES (7, 'x'), (8, 'y')", &[])?;
            Ok(())
        })
        .unwrap_err();
    let (code, message) = refusal(error);
    assert_eq!(code, "constraint");
    assert!(message.contains("posts.(user_id) → users: 2 rows"), "{message}");
    assert_eq!(posts(&zega), 3);
    assert_eq!(zega.sql("SELECT name FROM pragma_table_info('users')", &[]).unwrap().rows.len(), 2, "the rebuild rolled back");
    assert_eq!(int(zega.sql("SELECT * FROM pragma_foreign_keys", &[]).unwrap()), 1);
}

#[test]
fn a_migration_that_fails_turns_foreign_keys_back_on() {
    let dir = tempfile::tempdir().unwrap();
    let zega = open(dir.path().to_str().unwrap());
    users_and_posts(&zega);
    zega.migration(|tx| {
        tx.sql("DROP TABLE users", &[])?;
        Err::<(), _>(ZegaError::Execution("changed my mind".into()))
    })
    .unwrap_err();
    assert_eq!(posts(&zega), 3);
    assert_eq!(refusal(zega.sql("INSERT INTO posts (user_id) VALUES (999)", &[]).unwrap_err()).0, "constraint");
}

#[test]
fn the_catalog_reads_through_sql() {
    let dir = tempfile::tempdir().unwrap();
    let zega = open(dir.path().to_str().unwrap());
    users_and_posts(&zega);
    zega.sql("CREATE INDEX posts_user ON posts(user_id)", &[]).unwrap();
    // A schema sync reads its own types back from the declared type names.
    zega.sql("ALTER TABLE users ADD COLUMN email VARCHAR(120)", &[]).unwrap();
    zega.sql("ALTER TABLE users ADD COLUMN seen TIMESTAMP_MS", &[]).unwrap();
    let tables = zega
        .sql(
            "SELECT name, sql FROM sqlite_schema WHERE type = 'table' AND name NOT LIKE 'zega\\_%' ESCAPE '\\' ORDER BY name",
            &[],
        )
        .unwrap();
    assert_eq!(tables.rows.len(), 2);
    // turso keeps the definition, re-rendered: "users (id)", not "users(id)".
    assert!(matches!(&tables.rows[1][1], SqlValue::Text(sql) if sql.contains("email VARCHAR(120)") && sql.contains("seen TIMESTAMP_MS")), "{:?}", tables.rows);
    let info = zega.sql("SELECT name, type, \"notnull\", pk FROM pragma_table_info('users') ORDER BY cid", &[]).unwrap();
    assert_eq!(info.rows[1], vec![SqlValue::Text("name".into()), SqlValue::Text("TEXT".into()), SqlValue::Integer(1), SqlValue::Integer(0)]);
    // Unlike SQLite, turso's table_info drops a type's size: read full type
    // names from sqlite_schema.sql (above), which keeps "VARCHAR(120)".
    assert_eq!(info.rows[2][1], SqlValue::Text("VARCHAR".into()));
    assert_eq!(info.rows[3][1], SqlValue::Text("TIMESTAMP_MS".into()));
    let keys = zega.sql("SELECT \"table\", \"from\", \"to\", on_delete FROM pragma_foreign_key_list('posts')", &[]).unwrap();
    assert_eq!(
        keys.rows,
        vec![vec![SqlValue::Text("users".into()), SqlValue::Text("user_id".into()), SqlValue::Text("id".into()), SqlValue::Text("CASCADE".into())]]
    );
    let indexes = zega.sql("SELECT name FROM pragma_index_list('posts')", &[]).unwrap();
    assert_eq!(indexes.rows, vec![vec![SqlValue::Text("posts_user".into())]]);
}

#[test]
fn expression_defaults_returning_and_json() {
    let dir = tempfile::tempdir().unwrap();
    let zega = open(dir.path().to_str().unwrap());
    users_and_posts(&zega);
    zega.sql("CREATE TABLE events (id INTEGER PRIMARY KEY, at INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)))", &[])
        .unwrap();
    let inserted = zega.sql("INSERT INTO events DEFAULT VALUES RETURNING id, at", &[]).unwrap();
    assert_eq!(inserted.rows[0][0], SqlValue::Integer(1));
    assert!(matches!(inserted.rows[0][1], SqlValue::Integer(ms) if ms > 1_700_000_000_000), "{:?}", inserted.rows);
    let updated = zega.sql("UPDATE users SET name = upper(name) WHERE id = 1 RETURNING *", &[]).unwrap();
    assert_eq!(updated.rows, vec![vec![SqlValue::Integer(1), SqlValue::Text("ADA".into())]]);
    let nested = zega
        .sql(
            "SELECT json_object('name', u.name, 'posts', (SELECT json_group_array(p.title) FROM posts p WHERE p.user_id = u.id ORDER BY p.id)) \
             FROM users u WHERE u.id IN (SELECT value FROM json_each(?1)) ORDER BY u.id",
            &[SqlValue::Text("[1, 9]".into())],
        )
        .unwrap();
    assert_eq!(nested.rows, vec![vec![SqlValue::Text(r#"{"name":"ADA","posts":["a","b"]}"#.into())]]);
}

#[test]
fn several_ddl_steps_commit_together_or_not_at_all() {
    let dir = tempfile::tempdir().unwrap();
    let zega = open(dir.path().to_str().unwrap());
    users_and_posts(&zega);
    zega.transaction(|tx| {
        tx.sql("ALTER TABLE users ADD COLUMN bio TEXT", &[])?;
        tx.sql("ALTER TABLE users RENAME COLUMN bio TO about", &[])?;
        tx.sql("CREATE TABLE tags (id INTEGER PRIMARY KEY, name TEXT NOT NULL)", &[])?;
        tx.sql("CREATE INDEX tags_name ON tags(name)", &[])?;
        Ok(())
    })
    .unwrap();
    zega.transaction(|tx| {
        tx.sql("CREATE TABLE doomed (id INTEGER PRIMARY KEY)", &[])?;
        tx.sql("DROP TABLE tags", &[])?;
        tx.sql("INSERT INTO nowhere VALUES (1)", &[])?;
        Ok(())
    })
    .unwrap_err();
    let names = zega
        .sql("SELECT name FROM sqlite_schema WHERE name IN ('doomed', 'tags', 'tags_name') ORDER BY name", &[])
        .unwrap();
    assert_eq!(names.rows, vec![vec![SqlValue::Text("tags".into())], vec![SqlValue::Text("tags_name".into())]]);
    let about = zega.sql("SELECT count(*) FROM pragma_table_info('users') WHERE name = 'about'", &[]).unwrap();
    assert_eq!(int(about), 1);
}
