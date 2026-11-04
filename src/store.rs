//! The SQLite database every glos process shares: the UI, MCP servers and the CLI.
//!
//! Every action is its own transaction, written at once. WAL mode and a busy timeout let several
//! processes write without overwriting each other. The schema is versioned with
//! `PRAGMA user_version`; migrations run at open, after copying the database to a backup.

use std::{path::Path, time::Duration};

use anyhow::{Context, Result};
use rusqlite::Connection;

/// Each entry moves the schema up one version. Never edit a released entry; add a new one.
const MIGRATIONS: &[&str] = &[
    // v1
    r#"
    CREATE TABLE reviews (
        id          TEXT PRIMARY KEY,
        -- 'files' | 'folders' | 'git'
        source_kind TEXT NOT NULL,
        -- What a review is found by: for git, the shared git dir + worktree + branch;
        -- for files and folders, the two canonical paths.
        source_key  TEXT NOT NULL UNIQUE,
        -- The folder the review belongs to (the worktree, or the folder holding the files):
        -- MCP servers registered for this folder get its submits.
        folder      TEXT NOT NULL,
        old_path    TEXT,
        new_path    TEXT,
        git_dir     TEXT,
        branch      TEXT,
        base        TEXT,
        -- 'open' | 'closed' | 'archived'
        status      TEXT NOT NULL DEFAULT 'open',
        created_at  TEXT NOT NULL,
        updated_at  TEXT NOT NULL
    );

    -- Each distinct file content once, by hash: snapshots and anchors refer to it.
    CREATE TABLE blobs (
        hash    TEXT PRIMARY KEY,
        content BLOB NOT NULL
    );

    CREATE TABLE rounds (
        review_id    TEXT NOT NULL REFERENCES reviews(id) ON DELETE CASCADE,
        n            INTEGER NOT NULL,
        submitted_at TEXT NOT NULL,
        -- 'approved' | 'changes' | 'comment'
        verdict      TEXT NOT NULL,
        -- git: the head commit at submit, for reference
        head_commit  TEXT,
        PRIMARY KEY (review_id, n)
    );

    -- What you reviewed in each round: every file in the review at submit.
    CREATE TABLE snapshot_files (
        review_id TEXT NOT NULL,
        round     INTEGER NOT NULL,
        path      TEXT NOT NULL,
        hash      TEXT NOT NULL REFERENCES blobs(hash),
        PRIMARY KEY (review_id, round, path),
        FOREIGN KEY (review_id, round) REFERENCES rounds(review_id, n) ON DELETE CASCADE
    );

    CREATE TABLE threads (
        id            TEXT PRIMARY KEY,
        review_id     TEXT NOT NULL REFERENCES reviews(id) ON DELETE CASCADE,
        -- The anchor; all NULL for a review-level thread. Lines are 1-based, inclusive, new side.
        path          TEXT,
        line_start    INTEGER,
        line_end      INTEGER,
        anchor_text   TEXT,
        context_above TEXT,
        context_below TEXT,
        file_hash     TEXT REFERENCES blobs(hash),
        -- 'open' | 'resolved' | 'outdated'
        status        TEXT NOT NULL DEFAULT 'open',
        created_round INTEGER NOT NULL,
        last_seen_at  TEXT
    );
    CREATE INDEX threads_review ON threads(review_id);

    CREATE TABLE comments (
        id           TEXT PRIMARY KEY,
        thread_id    TEXT NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
        -- 'you', or the agent's name
        author       TEXT NOT NULL,
        -- 'comment' | 'annotation'
        kind         TEXT NOT NULL DEFAULT 'comment',
        body         TEXT NOT NULL,
        created_at   TEXT NOT NULL,
        edited_at    TEXT,
        -- The round it was sent in; NULL while unsubmitted.
        submitted_in INTEGER
    );
    CREATE INDEX comments_thread ON comments(thread_id);

    -- The text in an open comment box, saved as you type. One per place: a new comment on a line
    -- range, a review-level comment, a reply to a thread, or an edit of a comment.
    CREATE TABLE drafts (
        review_id  TEXT NOT NULL REFERENCES reviews(id) ON DELETE CASCADE,
        -- e.g. 'line:src/main.rs:10-12', 'review', 'reply:<thread id>', 'edit:<comment id>'
        place      TEXT NOT NULL,
        body       TEXT NOT NULL,
        updated_at TEXT NOT NULL,
        PRIMARY KEY (review_id, place)
    );

    -- One row per running `glos mcp`.
    CREATE TABLE sessions (
        pid        INTEGER PRIMARY KEY,
        -- From the MCP client's clientInfo.
        agent_name TEXT NOT NULL,
        folder     TEXT NOT NULL,
        attached_at TEXT NOT NULL,
        last_seen  TEXT NOT NULL
    );
    "#,
    // v2: delivering submitted rounds to agents, and agents asking for review.
    r#"
    -- When an agent's wait_for_review took the round, and which agent.
    ALTER TABLE rounds ADD COLUMN delivered_at TEXT;
    ALTER TABLE rounds ADD COLUMN delivered_to TEXT;
    -- The open request_review, if any: when, by which agent, its note and files (one per line).
    ALTER TABLE reviews ADD COLUMN requested_at TEXT;
    ALTER TABLE reviews ADD COLUMN requested_by TEXT;
    ALTER TABLE reviews ADD COLUMN request_note TEXT;
    ALTER TABLE reviews ADD COLUMN request_files TEXT;
    "#,
    // v3: an agent's suggestion of a line for you to look at (one per review, the latest).
    r#"
    ALTER TABLE reviews ADD COLUMN suggested_at TEXT;
    ALTER TABLE reviews ADD COLUMN suggested_by TEXT;
    ALTER TABLE reviews ADD COLUMN suggested_path TEXT;
    ALTER TABLE reviews ADD COLUMN suggested_line INTEGER;
    ALTER TABLE reviews ADD COLUMN suggested_note TEXT;
    "#,
    // v4: the diff each thread was made on, so an agent knows what code a comment is about.
    r#"
    -- The commits on each side (NULL for the files on disk, or a non-git side).
    ALTER TABLE threads ADD COLUMN old_rev TEXT;
    ALTER TABLE threads ADD COLUMN new_rev TEXT;
    -- What was compared, said for a reader: e.g. 'commit 2/3 d0c4165 "add b" vs its parent'.
    ALTER TABLE threads ADD COLUMN diff TEXT;
    "#,
];

/// Opens (creating if needed) the database at `path` and brings its schema up to date.
pub fn open(path: &Path) -> Result<Connection> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let mut conn = Connection::open(path).with_context(|| format!("opening {}", path.display()))?;
    conn.busy_timeout(Duration::from_secs(5))?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", true)?;
    migrate(&mut conn, path, MIGRATIONS)?;
    Ok(conn)
}

fn migrate(conn: &mut Connection, path: &Path, migrations: &[&str]) -> Result<()> {
    let version = conn.pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))? as usize;
    anyhow::ensure!(
        version <= migrations.len(),
        "{} has schema version {version}, newer than this glos ({}); upgrade glos",
        path.display(),
        migrations.len()
    );
    if version == migrations.len() {
        return Ok(());
    }
    if version > 0 {
        let backup = path.with_extension(format!("db.bak-v{version}"));
        let _ = std::fs::remove_file(&backup);
        conn.execute("VACUUM INTO ?1", [backup.to_string_lossy()])
            .with_context(|| format!("backing up to {}", backup.display()))?;
    }
    // One transaction for all pending steps, so a failed migration leaves the old schema intact.
    let tx = conn.transaction()?;
    for (i, sql) in migrations.iter().enumerate().skip(version) {
        tx.execute_batch(sql)
            .with_context(|| format!("migrating to v{}", i + 1))?;
    }
    tx.pragma_update(None, "user_version", migrations.len() as u32)?;
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(conn: &Connection) -> usize {
        conn.pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
            .unwrap() as usize
    }

    #[test]
    fn opens_and_reopens() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub/glos.db");
        let conn = open(&path).unwrap();
        assert_eq!(version(&conn), MIGRATIONS.len());
        drop(conn);
        let conn = open(&path).unwrap();
        assert_eq!(version(&conn), MIGRATIONS.len());
        // A fresh database needs no backup.
        assert!(!dir.path().join("sub/glos.db.bak-v0").exists());
    }

    #[test]
    fn backs_up_before_migrating_and_refuses_newer() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("glos.db");
        let mut conn = Connection::open(&path).unwrap();
        migrate(&mut conn, &path, &["CREATE TABLE a (x)"]).unwrap();
        conn.execute("INSERT INTO a VALUES (1)", []).unwrap();
        migrate(
            &mut conn,
            &path,
            &["CREATE TABLE a (x)", "CREATE TABLE b (y)"],
        )
        .unwrap();
        assert_eq!(version(&conn), 2);

        let backup = Connection::open(dir.path().join("glos.db.bak-v1")).unwrap();
        assert_eq!(version(&backup), 1);
        let n: i64 = backup
            .query_row("SELECT count(*) FROM a", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);

        let err = migrate(&mut conn, &path, &["CREATE TABLE a (x)"]).unwrap_err();
        assert!(err.to_string().contains("newer than this glos"));
    }

    #[test]
    fn failed_migration_keeps_old_schema() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("glos.db");
        let mut conn = Connection::open(&path).unwrap();
        migrate(&mut conn, &path, &["CREATE TABLE a (x)"]).unwrap();
        assert!(
            migrate(
                &mut conn,
                &path,
                &["CREATE TABLE a (x)", "CREATE TABLE b (y); nonsense"]
            )
            .is_err()
        );
        assert_eq!(version(&conn), 1);
        let has_b: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name = 'b'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(has_b, 0);
    }

    #[test]
    fn schema_has_every_table_and_enforces_references() {
        let dir = tempfile::tempdir().unwrap();
        let conn = open(&dir.path().join("glos.db")).unwrap();
        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(
            tables,
            [
                "blobs",
                "comments",
                "drafts",
                "reviews",
                "rounds",
                "sessions",
                "snapshot_files",
                "threads"
            ]
        );
        // Foreign keys are on: a comment needs its thread.
        let err = conn.execute(
            "INSERT INTO comments (id, thread_id, author, body, created_at) VALUES ('c', 'nope', 'you', 'x', 'now')",
            [],
        );
        assert!(err.is_err());
        // Deleting a review deletes its threads and their comments.
        conn.execute_batch(
            "INSERT INTO reviews (id, source_kind, source_key, folder, created_at, updated_at)
                 VALUES ('r', 'files', 'k', '/f', 'now', 'now');
             INSERT INTO threads (id, review_id, created_round) VALUES ('t', 'r', 0);
             INSERT INTO comments (id, thread_id, author, body, created_at) VALUES ('c', 't', 'you', 'x', 'now');
             DELETE FROM reviews WHERE id = 'r';",
        )
        .unwrap();
        let n: i64 = conn
            .query_row("SELECT count(*) FROM comments", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn two_processes_write_without_losing_each_others_changes() {
        // Two connections stand in for the UI and an MCP server.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("glos.db");
        let a = open(&path).unwrap();
        let b = open(&path).unwrap();
        let mode: String = a
            .pragma_query_value(None, "journal_mode", |r| r.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
        let version = |c: &Connection| -> i64 {
            c.pragma_query_value(None, "data_version", |r| r.get(0))
                .unwrap()
        };
        let seen = version(&a);
        b.execute(
            "INSERT INTO reviews (id, source_kind, source_key, folder, created_at, updated_at)
             VALUES ('r', 'files', 'k', '/f', 'now', 'now')",
            [],
        )
        .unwrap();
        // `a` notices `b`'s write through data_version, which is how processes see changes.
        assert_ne!(version(&a), seen);
        a.execute("UPDATE reviews SET status = 'closed' WHERE id = 'r'", [])
            .unwrap();
        let status: String = b
            .query_row("SELECT status FROM reviews", [], |r| r.get(0))
            .unwrap();
        assert_eq!(status, "closed");
    }

    #[test]
    fn open_fails_clearly_when_the_folder_cannot_be_made() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("file");
        std::fs::write(&file, "x").unwrap();
        let err = open(&file.join("glos.db")).unwrap_err().to_string();
        assert!(err.contains("creating"), "{err}");
    }
}
