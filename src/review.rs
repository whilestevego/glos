//! The review model over the store: reviews, threads, comments, drafts, rounds and snapshots.
//! Every change is one transaction, written the moment it happens.

use std::path::Path;

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::Serialize;

use crate::source::Source;

/// The author of your own comments.
pub const YOU: &str = "you";

pub fn now() -> String {
    jiff::Timestamp::now().to_string()
}

/// Short random ids: agents quote thread ids back, so they're kept readable.
pub fn new_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..12].to_string()
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Review {
    pub id: String,
    /// `files`, `folders` or `git`.
    pub kind: String,
    pub folder: String,
    pub old_path: Option<String>,
    pub new_path: Option<String>,
    pub branch: Option<String>,
    pub base: Option<String>,
    /// `open`, `closed` or `archived`.
    pub status: String,
    /// An agent's open `request_review`: when, by whom, and its note.
    pub requested_at: Option<String>,
    pub requested_by: Option<String>,
    pub request_note: Option<String>,
    pub updated_at: String,
}

const REVIEW_COLS: &str = "id, source_kind, folder, old_path, new_path, branch, base, status, \
                           requested_at, requested_by, request_note, updated_at";

fn review_row(r: &Row) -> rusqlite::Result<Review> {
    Ok(Review {
        id: r.get(0)?,
        kind: r.get(1)?,
        folder: r.get(2)?,
        old_path: r.get(3)?,
        new_path: r.get(4)?,
        branch: r.get(5)?,
        base: r.get(6)?,
        status: r.get(7)?,
        requested_at: r.get(8)?,
        requested_by: r.get(9)?,
        request_note: r.get(10)?,
        updated_at: r.get(11)?,
    })
}

impl Review {
    /// The review of `source`, made the first time it's seen. Reviews are keyed by their source:
    /// for git the shared git dir, worktree and branch; for files and folders their two paths.
    pub fn open(conn: &Connection, source: &Source) -> Result<Review> {
        let s = |p: &Path| p.to_string_lossy().into_owned();
        let (kind, key, folder, old, new, git_dir, branch, base) =
            match (&source.git, &source.paths) {
                (Some(g), _) => {
                    // ponytail: detached HEAD is keyed as one branch; see PLAN's open questions.
                    let branch = g.branch.clone().unwrap_or_else(|| "(detached)".into());
                    let key = format!("git:{}:{}:{branch}", s(&g.common_dir), s(&g.worktree));
                    let git = (Some(s(&g.common_dir)), Some(branch), Some(g.base.clone()));
                    ("git", key, s(&g.worktree), None, None, git.0, git.1, git.2)
                }
                (None, Some((old, new))) => {
                    let kind = if new.is_dir() { "folders" } else { "files" };
                    let folder = if new.is_dir() {
                        new.as_path()
                    } else {
                        new.parent().unwrap_or(Path::new("/"))
                    };
                    let key = format!("{kind}:{}:{}", s(old), s(new));
                    let (o, n) = (Some(s(old)), Some(s(new)));
                    (kind, key, s(folder), o, n, None, None, None)
                }
                (None, None) => bail!("a source with neither git nor paths"),
            };
        let t = now();
        conn.execute(
            "INSERT INTO reviews (id, source_kind, source_key, folder, old_path, new_path, git_dir,
                                  branch, base, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)
             ON CONFLICT(source_key) DO UPDATE SET base = excluded.base",
            params![
                new_id(),
                kind,
                key,
                folder,
                old,
                new,
                git_dir,
                branch,
                base,
                t
            ],
        )?;
        conn.query_row(
            &format!("SELECT {REVIEW_COLS} FROM reviews WHERE source_key = ?1"),
            [key],
            review_row,
        )
        .context("reading the review back")
    }

    /// A review by its id, or by the start of it, as git takes short commit ids.
    pub fn get(conn: &Connection, id: &str) -> Result<Option<Review>> {
        let mut found = conn
            .prepare(&format!(
                "SELECT {REVIEW_COLS} FROM reviews WHERE substr(id, 1, length(?1)) = ?1"
            ))?
            .query_map([id], review_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if found.len() > 1 {
            bail!("{id} matches {} reviews; give more of its id", found.len());
        }
        Ok(found.pop())
    }

    /// Open reviews by most recently changed; with `all`, closed and archived ones too.
    pub fn list(conn: &Connection, all: bool) -> Result<Vec<Review>> {
        let filter = if all { "" } else { "WHERE status = 'open'" };
        Ok(conn
            .prepare(&format!(
                "SELECT {REVIEW_COLS} FROM reviews {filter} ORDER BY updated_at DESC, rowid DESC"
            ))?
            .query_map([], review_row)?
            .collect::<rusqlite::Result<_>>()?)
    }

    /// The open review whose folder holds `dir`: the deepest folder, then the newest review.
    /// This is how an MCP server or the CLI finds the review it serves.
    pub fn for_folder(conn: &Connection, dir: &Path) -> Result<Option<Review>> {
        Ok(Self::list(conn, false)?
            .into_iter()
            .filter(|r| dir.starts_with(&r.folder))
            .max_by(|a, b| (a.folder.len(), &a.updated_at).cmp(&(b.folder.len(), &b.updated_at))))
    }

    pub fn reload(&self, conn: &Connection) -> Result<Review> {
        Self::get(conn, &self.id)?.context("the review was deleted")
    }

    /// The review's changes as they are now, found the way `glos` or `glos diff` finds them.
    pub fn source(&self) -> Result<Source> {
        match (self.kind.as_str(), &self.old_path, &self.new_path) {
            ("git", _, _) => Source::git(Path::new(&self.folder), self.base.as_deref()),
            (_, Some(old), Some(new)) => Source::diff(Path::new(old), Path::new(new)),
            _ => bail!("review {} has no paths", self.id),
        }
    }

    /// One line saying what's reviewed: `branch topic (vs main) in /repo`, or `old → new`.
    pub fn describe(&self) -> String {
        match (&self.branch, &self.old_path, &self.new_path) {
            (Some(b), _, _) => format!(
                "branch {b} (vs {}) in {}",
                self.base.as_deref().unwrap_or("?"),
                self.folder
            ),
            (_, Some(o), Some(n)) => format!("{o} → {n}"),
            _ => self.folder.clone(),
        }
    }

    /// An agent says its work is ready, optionally naming the files it touched.
    pub fn request(&self, conn: &Connection, by: &str, note: &str, files: &[String]) -> Result<()> {
        conn.execute(
            "UPDATE reviews SET requested_at = ?2, requested_by = ?3, request_note = ?4,
                    request_files = ?5, updated_at = ?2
             WHERE id = ?1",
            params![self.id, now(), by, note, files.join("\n")],
        )?;
        Ok(())
    }
}

/// Where a thread sits: lines `start..=end` (1-based) of `path`, on the new side.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct Anchor {
    pub path: String,
    pub start: u32,
    pub end: u32,
}

impl std::fmt::Display for Anchor {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        if self.start == self.end {
            write!(f, "{}:{}", self.path, self.start)
        } else {
            write!(f, "{}:{}-{}", self.path, self.start, self.end)
        }
    }
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Thread {
    pub id: String,
    /// `None` for a review-level thread.
    pub anchor: Option<Anchor>,
    /// `open`, `resolved` or `outdated`.
    pub status: String,
    pub created_round: u32,
    /// The anchored lines' text when the thread was made.
    pub anchor_text: Option<String>,
    pub comments: Vec<Comment>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Comment {
    pub id: String,
    /// [`YOU`], or the agent's name.
    pub author: String,
    /// `comment`, or `annotation` for an agent's note made before review.
    pub kind: String,
    pub body: String,
    pub created_at: String,
    pub edited_at: Option<String>,
    /// The round it was sent in; `None` while unsent.
    pub submitted_in: Option<u32>,
}

impl Comment {
    /// Yours and not yet sent to the agent.
    pub fn unsent(&self) -> bool {
        self.author == YOU && self.submitted_in.is_none()
    }
}

impl Thread {
    /// Your latest comment in the thread, which `space m` edits.
    pub fn latest_yours(&self) -> Option<&Comment> {
        self.comments.iter().rev().find(|c| c.author == YOU)
    }
}

/// Every thread of a review with its comments: review-level threads first, then by file and line.
pub fn threads(conn: &Connection, review_id: &str) -> Result<Vec<Thread>> {
    let mut threads: Vec<Thread> = conn
        .prepare(
            "SELECT id, path, line_start, line_end, status, created_round, anchor_text
             FROM threads WHERE review_id = ?1 ORDER BY path, line_start, rowid",
        )?
        .query_map([review_id], |r| {
            let path: Option<String> = r.get(1)?;
            Ok(Thread {
                id: r.get(0)?,
                anchor: match path {
                    Some(path) => Some(Anchor {
                        path,
                        start: r.get(2)?,
                        end: r.get(3)?,
                    }),
                    None => None,
                },
                status: r.get(4)?,
                created_round: r.get(5)?,
                anchor_text: r.get(6)?,
                comments: vec![],
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    let mut stmt = conn.prepare(
        "SELECT id, author, kind, body, created_at, edited_at, submitted_in
         FROM comments WHERE thread_id = ?1 ORDER BY created_at, rowid",
    )?;
    for t in &mut threads {
        t.comments = stmt
            .query_map([&t.id], |r| {
                Ok(Comment {
                    id: r.get(0)?,
                    author: r.get(1)?,
                    kind: r.get(2)?,
                    body: r.get(3)?,
                    created_at: r.get(4)?,
                    edited_at: r.get(5)?,
                    submitted_in: r.get(6)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
    }
    Ok(threads)
}

fn next_round(conn: &Connection, review_id: &str) -> Result<u32> {
    Ok(conn.query_row(
        "SELECT coalesce(max(n), 0) + 1 FROM rounds WHERE review_id = ?1",
        [review_id],
        |r| r.get(0),
    )?)
}

fn touch(conn: &Connection, review_id: &str) -> Result<()> {
    conn.execute(
        "UPDATE reviews SET updated_at = ?2 WHERE id = ?1",
        params![review_id, now()],
    )?;
    Ok(())
}

/// Starts a thread with one comment. `file` is the anchored file's content now: its lines,
/// the three lines around them and the whole content are kept for re-anchoring.
pub fn comment(
    conn: &Connection,
    review_id: &str,
    anchor: Option<(&Anchor, &str)>,
    author: &str,
    body: &str,
) -> Result<String> {
    let tx = conn.unchecked_transaction()?;
    let id = new_id();
    let round = next_round(&tx, review_id)?;
    match anchor {
        Some((a, file)) => {
            let lines: Vec<&str> = file.split_inclusive('\n').collect();
            let (s, e) = (a.start as usize - 1, (a.end as usize).min(lines.len()));
            let text = |r: std::ops::Range<usize>| lines.get(r).unwrap_or_default().concat();
            let hash = put_blob(&tx, file.as_bytes())?;
            tx.execute(
                "INSERT INTO threads (id, review_id, path, line_start, line_end, anchor_text,
                                      context_above, context_below, file_hash, created_round)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    id,
                    review_id,
                    a.path,
                    a.start,
                    a.end,
                    text(s.min(e)..e),
                    text(s.saturating_sub(3).min(e)..s.min(e)),
                    text(e..(e + 3).min(lines.len())),
                    hash,
                    round
                ],
            )?;
        }
        None => {
            tx.execute(
                "INSERT INTO threads (id, review_id, created_round) VALUES (?1, ?2, ?3)",
                params![id, review_id, round],
            )?;
        }
    }
    add_comment(&tx, &id, author, body)?;
    touch(&tx, review_id)?;
    tx.commit()?;
    Ok(id)
}

fn add_comment(conn: &Connection, thread_id: &str, author: &str, body: &str) -> Result<String> {
    let id = new_id();
    conn.execute(
        "INSERT INTO comments (id, thread_id, author, body, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![id, thread_id, author, body, now()],
    )?;
    Ok(id)
}

fn review_of_thread(conn: &Connection, thread_id: &str) -> Result<String> {
    conn.query_row(
        "SELECT review_id FROM threads WHERE id = ?1",
        [thread_id],
        |r| r.get(0),
    )
    .optional()?
    .with_context(|| format!("no thread {thread_id}"))
}

pub fn reply(conn: &Connection, thread_id: &str, author: &str, body: &str) -> Result<String> {
    let tx = conn.unchecked_transaction()?;
    let review = review_of_thread(&tx, thread_id)?;
    let id = add_comment(&tx, thread_id, author, body)?;
    touch(&tx, &review)?;
    tx.commit()?;
    Ok(id)
}

/// Changes a comment. Before it's sent this is silent; after, the comment is marked edited and
/// goes to the agent again with the next submit.
pub fn edit(conn: &Connection, comment_id: &str, body: &str) -> Result<()> {
    let n = conn.execute(
        "UPDATE comments SET body = ?2,
                edited_at = CASE WHEN submitted_in IS NULL THEN edited_at ELSE ?3 END,
                submitted_in = NULL
         WHERE id = ?1",
        params![comment_id, body, now()],
    )?;
    if n == 0 {
        bail!("no comment {comment_id}");
    }
    Ok(())
}

/// Deletes an unsent comment, and its thread if nothing is left in it. A comment the agent has
/// already seen can only be edited.
pub fn delete(conn: &Connection, comment_id: &str) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    let (thread, sent): (String, Option<u32>) = tx
        .query_row(
            "SELECT thread_id, submitted_in FROM comments WHERE id = ?1",
            [comment_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
        .with_context(|| format!("no comment {comment_id}"))?;
    if sent.is_some() {
        bail!("that comment was already sent to the agent; edit it instead");
    }
    tx.execute("DELETE FROM comments WHERE id = ?1", [comment_id])?;
    tx.execute(
        "DELETE FROM threads WHERE id = ?1
           AND NOT EXISTS (SELECT 1 FROM comments WHERE thread_id = ?1)",
        [&thread],
    )?;
    tx.commit()?;
    Ok(())
}

/// How many of your comments the agent hasn't received.
pub fn unsent(conn: &Connection, review_id: &str) -> Result<usize> {
    Ok(conn.query_row(
        "SELECT count(*) FROM comments c JOIN threads t ON t.id = c.thread_id
         WHERE t.review_id = ?1 AND c.author = ?2 AND c.submitted_in IS NULL",
        params![review_id, YOU],
        |r| r.get::<_, i64>(0),
    )? as usize)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Approved,
    Changes,
    Comment,
}

impl Verdict {
    /// As stored, and as `:submit` and `glos submit` take it.
    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::Approved => "approve",
            Verdict::Changes => "changes",
            Verdict::Comment => "comment",
        }
    }

    pub fn parse(s: &str) -> Option<Verdict> {
        Some(match s {
            "approve" | "approved" | "lgtm" => Verdict::Approved,
            "changes" | "request-changes" => Verdict::Changes,
            "comment" | "comments" => Verdict::Comment,
            _ => return None,
        })
    }

    pub fn describe(self) -> &'static str {
        match self {
            Verdict::Approved => "approved",
            Verdict::Changes => "changes requested",
            Verdict::Comment => "comments",
        }
    }
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Round {
    pub n: u32,
    pub verdict: Verdict,
    pub submitted_at: String,
    /// How many comments were sent in it.
    pub comments: usize,
    pub delivered_to: Option<String>,
}

pub fn rounds(conn: &Connection, review_id: &str) -> Result<Vec<Round>> {
    let rows: Vec<(u32, String, String, Option<String>)> = conn
        .prepare(
            "SELECT n, verdict, submitted_at, delivered_to FROM rounds
             WHERE review_id = ?1 ORDER BY n",
        )?
        .query_map([review_id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    rows.into_iter()
        .map(|(n, verdict, submitted_at, delivered_to)| {
            let comments = conn.query_row(
                "SELECT count(*) FROM comments c JOIN threads t ON t.id = c.thread_id
                 WHERE t.review_id = ?1 AND c.submitted_in = ?2",
                params![review_id, n],
                |r| r.get::<_, i64>(0),
            )? as usize;
            Ok(Round {
                n,
                verdict: Verdict::parse(&verdict).context("a bad verdict in the store")?,
                submitted_at,
                comments,
                delivered_to,
            })
        })
        .collect()
}

/// Sends every unsent comment as one round with `verdict`, and snapshots `files` (path and
/// content on the new side) as what was reviewed. An approval needs no comments; the rest do.
pub fn submit(
    conn: &Connection,
    review_id: &str,
    verdict: Verdict,
    files: &[(String, Vec<u8>)],
    head: Option<&str>,
) -> Result<Round> {
    let tx = conn.unchecked_transaction()?;
    let count = unsent(&tx, review_id)?;
    if count == 0 && verdict != Verdict::Approved {
        bail!("nothing to submit: write a comment first, or approve");
    }
    let n = next_round(&tx, review_id)?;
    let at = now();
    tx.execute(
        "INSERT INTO rounds (review_id, n, submitted_at, verdict, head_commit)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![review_id, n, at, verdict.as_str(), head],
    )?;
    for (path, content) in files {
        let hash = put_blob(&tx, content)?;
        tx.execute(
            "INSERT INTO snapshot_files (review_id, round, path, hash) VALUES (?1, ?2, ?3, ?4)",
            params![review_id, n, path, hash],
        )?;
    }
    tx.execute(
        "UPDATE comments SET submitted_in = ?2
         WHERE submitted_in IS NULL AND author = ?3
           AND thread_id IN (SELECT id FROM threads WHERE review_id = ?1)",
        params![review_id, n, YOU],
    )?;
    // The agent's request is answered.
    tx.execute(
        "UPDATE reviews SET requested_at = NULL, requested_by = NULL, request_note = NULL,
                request_files = NULL, updated_at = ?2
         WHERE id = ?1",
        params![review_id, at],
    )?;
    tx.commit()?;
    Ok(Round {
        n,
        verdict,
        submitted_at: at,
        comments: count,
        delivered_to: None,
    })
}

/// The oldest submitted round no agent has received yet, with the threads that had comments
/// sent in it (only those comments). Marked delivered to `to`, so it's handed over once even
/// when several agents wait at the same time.
pub fn take_delivery(
    conn: &Connection,
    review_id: &str,
    to: &str,
) -> Result<Option<(Round, Vec<Thread>)>> {
    let tx = conn.unchecked_transaction()?;
    let Some(n): Option<u32> = tx
        .query_row(
            "SELECT n FROM rounds WHERE review_id = ?1 AND delivered_at IS NULL
             ORDER BY n LIMIT 1",
            [review_id],
            |r| r.get(0),
        )
        .optional()?
    else {
        return Ok(None);
    };
    let taken = tx.execute(
        "UPDATE rounds SET delivered_at = ?3, delivered_to = ?4
         WHERE review_id = ?1 AND n = ?2 AND delivered_at IS NULL",
        params![review_id, n, now(), to],
    )?;
    tx.commit()?;
    if taken == 0 {
        return Ok(None);
    }
    let round = rounds(conn, review_id)?
        .into_iter()
        .find(|r| r.n == n)
        .context("the round vanished")?;
    let threads = threads(conn, review_id)?
        .into_iter()
        .filter_map(|mut t| {
            t.comments.retain(|c| c.submitted_in == Some(n));
            (!t.comments.is_empty()).then_some(t)
        })
        .collect();
    Ok(Some((round, threads)))
}

/// The comment box's text for a place, saved as you type (see [`save_draft`]).
pub fn draft(conn: &Connection, review_id: &str, place: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT body FROM drafts WHERE review_id = ?1 AND place = ?2",
            [review_id, place],
            |r| r.get(0),
        )
        .optional()?)
}

/// Saves a comment box's text; an empty box has no draft.
pub fn save_draft(conn: &Connection, review_id: &str, place: &str, body: &str) -> Result<()> {
    if body.trim().is_empty() {
        conn.execute(
            "DELETE FROM drafts WHERE review_id = ?1 AND place = ?2",
            [review_id, place],
        )?;
    } else {
        conn.execute(
            "INSERT INTO drafts (review_id, place, body, updated_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(review_id, place) DO UPDATE SET body = ?3, updated_at = ?4",
            params![review_id, place, body, now()],
        )?;
    }
    Ok(())
}

/// Stores content once by its git blob hash, so snapshots and anchors share it.
fn put_blob(conn: &Connection, content: &[u8]) -> Result<String> {
    let hash =
        gix::objs::compute_hash(gix::hash::Kind::Sha1, gix::objs::Kind::Blob, content)?.to_string();
    conn.execute(
        "INSERT OR IGNORE INTO blobs (hash, content) VALUES (?1, ?2)",
        params![hash, content],
    )?;
    Ok(hash)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store;

    struct Fx {
        conn: Connection,
        review: Review,
        dir: tempfile::TempDir,
    }

    fn fx() -> Fx {
        let dir = tempfile::tempdir().unwrap();
        let (o, n) = (dir.path().join("old.rs"), dir.path().join("new.rs"));
        std::fs::write(&o, "a\n").unwrap();
        std::fs::write(&n, "a\nb\nc\nd\ne\nf\ng\nh\n").unwrap();
        let conn = store::open(&dir.path().join("glos.db")).unwrap();
        let review = Review::open(&conn, &Source::diff(&o, &n).unwrap()).unwrap();
        Fx { conn, review, dir }
    }

    fn at(path: &str, start: u32, end: u32) -> Anchor {
        Anchor {
            path: path.into(),
            start,
            end,
        }
    }

    const FILE: &str = "a\nb\nc\nd\ne\nf\ng\nh\n";

    #[test]
    fn a_source_has_one_review() {
        let f = fx();
        let source = f.review.source().unwrap();
        let again = Review::open(&f.conn, &source).unwrap();
        assert_eq!(again.id, f.review.id);
        assert_eq!(f.review.kind, "files");
        assert_eq!(f.review.status, "open");
        assert!(
            f.review.describe().contains(" → "),
            "{}",
            f.review.describe()
        );
        // The folder holding the files is the review's folder.
        let dir = f.dir.path().canonicalize().unwrap();
        assert_eq!(Path::new(&f.review.folder), dir);
        assert_eq!(
            Review::for_folder(&f.conn, &dir.join("sub")).unwrap(),
            Some(f.review.clone())
        );
        assert_eq!(Review::for_folder(&f.conn, Path::new("/")).unwrap(), None);
        assert_eq!(Review::list(&f.conn, false).unwrap().len(), 1);
    }

    #[test]
    fn reviews_are_found_by_short_id() {
        let f = fx();
        let short = &f.review.id[..4];
        assert_eq!(Review::get(&f.conn, short).unwrap(), Some(f.review.clone()));
        assert_eq!(Review::get(&f.conn, "zzzz").unwrap(), None);
        // A second review sharing no prefix with the first is told apart; "" matches both.
        let (o, n) = (f.dir.path().join("x"), f.dir.path().join("y"));
        std::fs::write(&o, "1").unwrap();
        std::fs::write(&n, "2").unwrap();
        Review::open(&f.conn, &Source::diff(&o, &n).unwrap()).unwrap();
        assert!(
            Review::get(&f.conn, "")
                .unwrap_err()
                .to_string()
                .contains("2 reviews")
        );
    }

    #[test]
    fn the_deepest_folder_wins() {
        let f = fx();
        let sub = f.dir.path().join("sub");
        std::fs::create_dir_all(sub.join("o")).unwrap();
        std::fs::create_dir_all(sub.join("n")).unwrap();
        std::fs::write(sub.join("n/x"), "x").unwrap();
        let inner = Review::open(
            &f.conn,
            &Source::diff(&sub.join("o"), &sub.join("n")).unwrap(),
        )
        .unwrap();
        assert_eq!(inner.kind, "folders");
        let deep = sub.canonicalize().unwrap().join("n/deeper");
        assert_eq!(
            Review::for_folder(&f.conn, &deep).unwrap().map(|r| r.id),
            Some(inner.id)
        );
    }

    #[test]
    fn comments_keep_their_anchor_and_context() {
        let f = fx();
        let t = comment(
            &f.conn,
            &f.review.id,
            Some((&at("new.rs", 5, 6), FILE)),
            YOU,
            "why?",
        )
        .unwrap();
        let (above, below, hash): (String, String, String) = f
            .conn
            .query_row(
                "SELECT context_above, context_below, file_hash FROM threads WHERE id = ?1",
                [&t],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!((above.as_str(), below.as_str()), ("b\nc\nd\n", "g\nh\n"));
        let content: Vec<u8> = f
            .conn
            .query_row("SELECT content FROM blobs WHERE hash = ?1", [&hash], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(content, FILE.as_bytes());
        let threads = threads(&f.conn, &f.review.id).unwrap();
        assert_eq!(threads.len(), 1);
        let th = &threads[0];
        assert_eq!(th.anchor, Some(at("new.rs", 5, 6)));
        assert_eq!(th.anchor_text.as_deref(), Some("e\nf\n"));
        assert_eq!((th.status.as_str(), th.created_round), ("open", 1));
        assert_eq!(th.comments[0].body, "why?");
        assert!(th.comments[0].unsent());
        // At the edges of the file the context is shorter, not out of range.
        comment(
            &f.conn,
            &f.review.id,
            Some((&at("new.rs", 1, 1), FILE)),
            YOU,
            "x",
        )
        .unwrap();
        comment(
            &f.conn,
            &f.review.id,
            Some((&at("new.rs", 8, 9), FILE)),
            YOU,
            "y",
        )
        .unwrap();
        assert_eq!(unsent(&f.conn, &f.review.id).unwrap(), 3);
    }

    #[test]
    fn threads_come_review_level_first_then_by_place() {
        let f = fx();
        let id = &f.review.id;
        comment(&f.conn, id, Some((&at("b.rs", 3, 3), FILE)), YOU, "b3").unwrap();
        comment(&f.conn, id, Some((&at("a.rs", 7, 7), FILE)), YOU, "a7").unwrap();
        comment(&f.conn, id, Some((&at("b.rs", 1, 2), FILE)), YOU, "b1").unwrap();
        comment(&f.conn, id, None, YOU, "overall").unwrap();
        let order: Vec<String> = threads(&f.conn, id)
            .unwrap()
            .iter()
            .map(|t| t.comments[0].body.clone())
            .collect();
        assert_eq!(order, ["overall", "a7", "b1", "b3"]);
    }

    #[test]
    fn edit_and_delete_before_and_after_submit() {
        let f = fx();
        let id = &f.review.id;
        let t = comment(&f.conn, id, None, YOU, "first").unwrap();
        let c = threads(&f.conn, id).unwrap()[0].comments[0].id.clone();
        edit(&f.conn, &c, "better").unwrap();
        let get = || threads(&f.conn, id).unwrap()[0].comments[0].clone();
        assert_eq!((get().body.as_str(), get().edited_at), ("better", None));

        submit(&f.conn, id, Verdict::Comment, &[], None).unwrap();
        assert_eq!(unsent(&f.conn, id).unwrap(), 0);
        let err = delete(&f.conn, &c).unwrap_err().to_string();
        assert!(err.contains("already sent"), "{err}");
        // Editing a sent comment marks it and sends it again with the next round.
        edit(&f.conn, &c, "even better").unwrap();
        assert!(get().edited_at.is_some() && get().unsent());
        assert_eq!(unsent(&f.conn, id).unwrap(), 1);

        // A reply, then deleting it, keeps the thread; deleting its only comment removes it.
        let r = reply(&f.conn, &t, YOU, "more").unwrap();
        delete(&f.conn, &r).unwrap();
        assert_eq!(threads(&f.conn, id).unwrap().len(), 1);
        let t2 = comment(&f.conn, id, None, YOU, "gone soon").unwrap();
        let c2 = threads(&f.conn, id)
            .unwrap()
            .into_iter()
            .find(|th| th.id == t2)
            .unwrap()
            .comments[0]
            .id
            .clone();
        delete(&f.conn, &c2).unwrap();
        assert_eq!(threads(&f.conn, id).unwrap().len(), 1);
        assert!(edit(&f.conn, "nope", "x").is_err());
        assert!(reply(&f.conn, "nope", YOU, "x").is_err());
    }

    #[test]
    fn submitting_sends_the_unsent_and_snapshots_files() {
        let f = fx();
        let id = &f.review.id;
        let err = submit(&f.conn, id, Verdict::Changes, &[], None).unwrap_err();
        assert!(err.to_string().contains("nothing to submit"));
        // An approval with nothing to say is an LGTM.
        let r1 = submit(
            &f.conn,
            id,
            Verdict::Approved,
            &[("a".into(), b"1".to_vec())],
            None,
        )
        .unwrap();
        assert_eq!((r1.n, r1.comments), (1, 0));

        comment(&f.conn, id, Some((&at("new.rs", 2, 2), FILE)), YOU, "x").unwrap();
        // An agent's comment is never "unsent".
        let t = comment(&f.conn, id, None, "claude", "note").unwrap();
        reply(&f.conn, &t, YOU, "ok").unwrap();
        assert_eq!(unsent(&f.conn, id).unwrap(), 2);
        let files = [("a".into(), b"1".to_vec()), ("b".into(), b"1".to_vec())];
        let r2 = submit(&f.conn, id, Verdict::Changes, &files, Some("abc")).unwrap();
        assert_eq!((r2.n, r2.comments, r2.verdict), (2, 2, Verdict::Changes));
        assert_eq!(unsent(&f.conn, id).unwrap(), 0);
        // Same content is stored once.
        let blobs: i64 = f
            .conn
            .query_row(
                "SELECT count(*) FROM blobs WHERE content = x'31'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(blobs, 1);
        let snap: i64 = f
            .conn
            .query_row(
                "SELECT count(*) FROM snapshot_files WHERE round = 2",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(snap, 2);
        let all = rounds(&f.conn, id).unwrap();
        assert_eq!(all.iter().map(|r| r.comments).collect::<Vec<_>>(), [0, 2]);
        // A new comment after submitting belongs to the next round.
        let t3 = comment(&f.conn, id, None, YOU, "later").unwrap();
        let th = threads(&f.conn, id).unwrap();
        assert_eq!(th.iter().find(|t| t.id == t3).unwrap().created_round, 3);
    }

    #[test]
    fn submitting_answers_the_request() {
        let f = fx();
        f.review
            .request(&f.conn, "claude", "done", &["a.rs".into()])
            .unwrap();
        let r = f.review.reload(&f.conn).unwrap();
        assert_eq!(r.requested_by.as_deref(), Some("claude"));
        assert_eq!(r.request_note.as_deref(), Some("done"));
        submit(&f.conn, &r.id, Verdict::Approved, &[], None).unwrap();
        assert_eq!(f.review.reload(&f.conn).unwrap().requested_at, None);
    }

    #[test]
    fn a_round_is_delivered_once_with_only_its_comments() {
        let f = fx();
        let id = &f.review.id;
        let other = store::open(&f.dir.path().join("glos.db")).unwrap();
        assert!(take_delivery(&other, id, "claude").unwrap().is_none());
        let t = comment(&f.conn, id, Some((&at("new.rs", 2, 2), FILE)), YOU, "one").unwrap();
        submit(&f.conn, id, Verdict::Changes, &[], None).unwrap();
        reply(&f.conn, &t, YOU, "two").unwrap();
        comment(&f.conn, id, None, YOU, "three").unwrap();
        submit(&f.conn, id, Verdict::Comment, &[], None).unwrap();

        let (r, threads) = take_delivery(&other, id, "claude").unwrap().unwrap();
        assert_eq!((r.n, r.verdict), (1, Verdict::Changes));
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].comments.len(), 1);
        let (r, threads) = take_delivery(&f.conn, id, "other").unwrap().unwrap();
        assert_eq!(r.n, 2);
        let bodies: Vec<&str> = threads
            .iter()
            .flat_map(|t| t.comments.iter().map(|c| c.body.as_str()))
            .collect();
        assert_eq!(bodies, ["three", "two"]);
        assert!(take_delivery(&other, id, "claude").unwrap().is_none());
        assert_eq!(
            rounds(&f.conn, id).unwrap()[0].delivered_to.as_deref(),
            Some("claude")
        );
    }

    #[test]
    fn drafts_are_kept_per_place() {
        let f = fx();
        let id = &f.review.id;
        assert_eq!(draft(&f.conn, id, "review").unwrap(), None);
        save_draft(&f.conn, id, "review", "hal").unwrap();
        save_draft(&f.conn, id, "review", "half a thought").unwrap();
        save_draft(&f.conn, id, "line:a:1-1", "other").unwrap();
        assert_eq!(
            draft(&f.conn, id, "review").unwrap().as_deref(),
            Some("half a thought")
        );
        save_draft(&f.conn, id, "review", "  \n").unwrap();
        assert_eq!(draft(&f.conn, id, "review").unwrap(), None);
        assert_eq!(
            draft(&f.conn, id, "line:a:1-1").unwrap().as_deref(),
            Some("other")
        );
    }

    #[test]
    fn verdicts_parse_and_round_trip() {
        for v in [Verdict::Approved, Verdict::Changes, Verdict::Comment] {
            assert_eq!(Verdict::parse(v.as_str()), Some(v));
        }
        assert_eq!(Verdict::parse("lgtm"), Some(Verdict::Approved));
        assert_eq!(Verdict::parse("nope"), None);
    }
}
