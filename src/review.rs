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
    /// An agent's latest `suggest`: a line for you to look at.
    pub suggestion: Option<Suggestion>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Suggestion {
    pub at: String,
    pub by: String,
    pub path: String,
    pub line: u32,
    pub note: String,
}

const REVIEW_COLS: &str = "id, source_kind, folder, old_path, new_path, branch, base, status, \
                           requested_at, requested_by, request_note, updated_at, \
                           suggested_at, suggested_by, suggested_path, suggested_line, \
                           suggested_note";

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
        suggestion: match r.get::<_, Option<String>>(12)? {
            Some(at) => Some(Suggestion {
                at,
                by: r.get(13)?,
                path: r.get(14)?,
                line: r.get(15)?,
                note: r.get::<_, Option<String>>(16)?.unwrap_or_default(),
            }),
            None => None,
        },
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
             ON CONFLICT(source_key) DO UPDATE SET base = excluded.base, status = 'open'",
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
        if id.is_empty() {
            bail!("no review id given");
        }
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

    pub fn reload(&self, conn: &Connection) -> Result<Review> {
        Self::get(conn, &self.id)?.context("the review was deleted")
    }

    /// The review's changes as they are now, found the way `glos` or `glos diff` finds them.
    /// A git review needs its worktree still on its branch.
    pub fn source(&self) -> Result<Source> {
        match (self.kind.as_str(), &self.old_path, &self.new_path) {
            ("git", _, _) => {
                let s = Source::git(Path::new(&self.folder), self.base.as_deref())?;
                let on = s.git.as_ref().and_then(|g| g.branch.clone());
                let on = on.unwrap_or_else(|| "(detached)".into());
                if Some(&on) != self.branch.as_ref() {
                    bail!(
                        "review {} is of branch {}, but {} is on {on} now",
                        self.id,
                        self.branch.as_deref().unwrap_or("?"),
                        self.folder
                    );
                }
                Ok(s)
            }
            (_, Some(old), Some(new)) => Source::diff(Path::new(old), Path::new(new)),
            _ => bail!("review {} has no paths", self.id),
        }
    }

    /// One line saying what's reviewed: `branch topic (vs main) in /repo`, `old → new`, or
    /// `a.rs → b.rs in /dir` for two files side by side.
    pub fn describe(&self) -> String {
        match (&self.branch, &self.old_path, &self.new_path) {
            (Some(b), _, _) => format!(
                "branch {b} (vs {}) in {}",
                self.base.as_deref().unwrap_or("?"),
                self.folder
            ),
            (_, Some(o), Some(n)) => {
                let (op, np) = (Path::new(o), Path::new(n));
                match (op.parent(), op.file_name(), np.file_name()) {
                    (Some(dir), Some(a), Some(b)) if np.parent() == Some(dir) => format!(
                        "{} → {} in {}",
                        a.to_string_lossy(),
                        b.to_string_lossy(),
                        dir.display()
                    ),
                    _ => format!("{o} → {n}"),
                }
            }
            _ => self.folder.clone(),
        }
    }

    /// An agent points you at a line; an open glos shows it, and `space v n` goes there.
    pub fn suggest(
        &self,
        conn: &Connection,
        by: &str,
        path: &str,
        line: u32,
        note: &str,
    ) -> Result<()> {
        conn.execute(
            "UPDATE reviews SET suggested_at = ?2, suggested_by = ?3, suggested_path = ?4,
                    suggested_line = ?5, suggested_note = ?6, updated_at = ?2
             WHERE id = ?1",
            params![self.id, now(), by, path, line, note],
        )?;
        Ok(())
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

/// `:close` and `glos close`: you're done with the review. Unsent comments must go first.
/// Running glos on its source again opens it again.
pub fn close(conn: &Connection, review_id: &str) -> Result<()> {
    let n = unsent(conn, review_id)?;
    if n > 0 {
        bail!(
            "{n} comment{} not sent to the agent: submit or delete {} first",
            if n == 1 { "" } else { "s" },
            if n == 1 { "it" } else { "them" }
        );
    }
    set_review_status(conn, review_id, "closed")
}

fn set_review_status(conn: &Connection, review_id: &str, status: &str) -> Result<()> {
    conn.execute(
        "UPDATE reviews SET status = ?2, updated_at = ?3 WHERE id = ?1",
        params![review_id, status, now()],
    )?;
    Ok(())
}

/// Archives the reviews that are over on their own: their folder is gone, their branch was
/// deleted, or it was merged into its base after you reviewed it, leaving nothing to review.
/// Returns the reviews archived.
pub fn archive_finished(conn: &Connection) -> Result<Vec<Review>> {
    let mut done = vec![];
    for r in Review::list(conn, true)? {
        if r.status == "archived" {
            continue;
        }
        let over = !Path::new(&r.folder).exists()
            || (r.kind == "git" && branch_over(conn, &r).unwrap_or(false));
        if over {
            set_review_status(conn, &r.id, "archived")?;
            done.push(r);
        }
    }
    Ok(done)
}

fn branch_over(conn: &Connection, r: &Review) -> Result<bool> {
    let (Some(branch), Some(base)) = (&r.branch, &r.base) else {
        return Ok(false);
    };
    if branch == "(detached)" {
        return Ok(false);
    }
    let repo = gix::open(&r.folder)?;
    let Some(mut tip) = repo.try_find_reference(&format!("refs/heads/{branch}"))? else {
        return Ok(true); // deleted
    };
    let tip = tip.peel_to_id()?.detach();
    let base_id = repo.rev_parse_single(base.as_str())?.detach();
    // Merged: the tip is in the base. A branch with no commits of its own is too, so it also
    // takes a round of review and nothing left to review.
    let merged = repo
        .merge_base(tip, base_id)
        .ok()
        .flatten()
        .map(|m| m.detach())
        == Some(tip);
    if !merged || rounds(conn, &r.id)?.is_empty() {
        return Ok(false);
    }
    let s = Source::git(Path::new(&r.folder), Some(base))?;
    let on = s.git.as_ref().and_then(|g| g.branch.as_deref());
    Ok(on != Some(branch.as_str()) || s.changes.is_empty())
}

/// `glos gc`: deletes archived reviews untouched for `older_than`, then file content nothing
/// refers to any more. Returns how many reviews and contents were deleted.
pub fn gc(conn: &Connection, older_than: jiff::SignedDuration) -> Result<(usize, usize)> {
    let cutoff = jiff::Timestamp::now() - older_than;
    let mut reviews = 0;
    for r in Review::list(conn, true)? {
        let old = r
            .updated_at
            .parse::<jiff::Timestamp>()
            .is_ok_and(|t| t < cutoff);
        if r.status == "archived" && old {
            reviews += conn.execute("DELETE FROM reviews WHERE id = ?1", [&r.id])?;
        }
    }
    let blobs = conn.execute(
        "DELETE FROM blobs WHERE hash NOT IN (SELECT hash FROM snapshot_files)
           AND hash NOT IN (SELECT file_hash FROM threads WHERE file_hash IS NOT NULL)",
        [],
    )?;
    conn.execute_batch("VACUUM")?;
    Ok((reviews, blobs))
}

/// Whether process `pid` is still running.
pub fn alive(pid: i64) -> bool {
    // SAFETY: signal 0 only checks that the process exists.
    let r = unsafe { libc::kill(pid as libc::pid_t, 0) };
    r == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// Records that this process shows `review_id` (or, with `None`, shows nothing any more), so
/// an agent's `request_review` knows glos is already open.
pub fn set_viewer(conn: &Connection, review_id: Option<&str>) -> Result<()> {
    let pid = std::process::id();
    match review_id {
        Some(id) => conn.execute(
            "INSERT INTO viewers (pid, review_id, started_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(pid) DO UPDATE SET review_id = ?2, started_at = ?3",
            params![pid, id, now()],
        )?,
        None => conn.execute("DELETE FROM viewers WHERE pid = ?1", [pid])?,
    };
    Ok(())
}

/// Whether a running glos shows `review_id`; forgets the ones whose process is gone.
pub fn viewing(conn: &Connection, review_id: &str) -> Result<bool> {
    let pids: Vec<i64> = conn
        .prepare("SELECT pid FROM viewers WHERE review_id = ?1")?
        .query_map([review_id], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let mut any = false;
    for pid in pids {
        if alive(pid) {
            any = true;
        } else {
            conn.execute("DELETE FROM viewers WHERE pid = ?1", [pid])?;
        }
    }
    Ok(any)
}

/// The diff a thread was made on: the commits on each side (`None` for the files on disk, or a
/// side that isn't git) and what was compared, said for a reader.
#[derive(Serialize, Clone, Debug, PartialEq, Default)]
pub struct DiffContext {
    pub old_rev: Option<String>,
    pub new_rev: Option<String>,
    pub diff: Option<String>,
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
    /// When you last opened it: agents' comments after this are unread.
    pub last_seen_at: Option<String>,
    /// The diff it was made on. A thread on your files follows them as they change; one made
    /// on a commit stays on that commit's code.
    #[serde(flatten)]
    pub on: DiffContext,
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
    /// Has comments from agents you haven't opened the thread to see.
    pub fn unread(&self) -> bool {
        self.comments.iter().any(|c| {
            c.author != YOU && self.last_seen_at.as_ref().is_none_or(|s| &c.created_at > s)
        })
    }

    /// Your latest comment in the thread, which `space v m` edits.
    pub fn latest_yours(&self) -> Option<&Comment> {
        self.comments.iter().rev().find(|c| c.author == YOU)
    }
}

/// Every thread of a review with its comments: review-level threads first, then by file and line.
pub fn threads(conn: &Connection, review_id: &str) -> Result<Vec<Thread>> {
    let mut threads: Vec<Thread> = conn
        .prepare(
            "SELECT id, path, line_start, line_end, status, created_round, anchor_text,
                    last_seen_at, old_rev, new_rev, diff
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
                last_seen_at: r.get(7)?,
                on: DiffContext {
                    old_rev: r.get(8)?,
                    new_rev: r.get(9)?,
                    diff: r.get(10)?,
                },
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

#[cfg(test)]
/// Starts a thread with one comment. `file` is the anchored file's content now: its lines,
/// the three lines around them and the whole content are kept for re-anchoring.
pub fn comment(
    conn: &Connection,
    review_id: &str,
    anchor: Option<(&Anchor, &str)>,
    author: &str,
    body: &str,
) -> Result<String> {
    comment_in(
        conn,
        review_id,
        anchor,
        author,
        body,
        &DiffContext::default(),
    )
}

/// [`comment`] on a diff: what was compared is kept with the thread.
pub fn comment_in(
    conn: &Connection,
    review_id: &str,
    anchor: Option<(&Anchor, &str)>,
    author: &str,
    body: &str,
    on: &DiffContext,
) -> Result<String> {
    comment_as(conn, review_id, anchor, author, "comment", body, on)
}

/// An agent's note on its own change, made before you review: a thread like a comment's, of
/// kind `annotation`.
pub fn annotate(
    conn: &Connection,
    review_id: &str,
    anchor: &Anchor,
    file: &str,
    author: &str,
    body: &str,
) -> Result<String> {
    let lines = file.split_inclusive('\n').count().max(1) as u32;
    if anchor.start == 0 || anchor.start > anchor.end || anchor.end > lines {
        bail!(
            "{} has {lines} lines; lines {}-{} aren't in it",
            anchor.path,
            anchor.start,
            anchor.end
        );
    }
    comment_as(
        conn,
        review_id,
        Some((anchor, file)),
        author,
        "annotation",
        body,
        &DiffContext::default(),
    )
}

fn comment_as(
    conn: &Connection,
    review_id: &str,
    anchor: Option<(&Anchor, &str)>,
    author: &str,
    kind: &str,
    body: &str,
    on: &DiffContext,
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
                                      context_above, context_below, file_hash, created_round,
                                      old_rev, new_rev, diff)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
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
                    round,
                    on.old_rev,
                    on.new_rev,
                    on.diff
                ],
            )?;
        }
        None => {
            tx.execute(
                "INSERT INTO threads (id, review_id, created_round, old_rev, new_rev, diff)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![id, review_id, round, on.old_rev, on.new_rev, on.diff],
            )?;
        }
    }
    add_comment(&tx, &id, author, kind, body)?;
    touch(&tx, review_id)?;
    tx.commit()?;
    Ok(id)
}

fn add_comment(
    conn: &Connection,
    thread_id: &str,
    author: &str,
    kind: &str,
    body: &str,
) -> Result<String> {
    let id = new_id();
    conn.execute(
        "INSERT INTO comments (id, thread_id, author, kind, body, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![id, thread_id, author, kind, body, now()],
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
    let id = add_comment(&tx, thread_id, author, "comment", body)?;
    touch(&tx, &review)?;
    tx.commit()?;
    Ok(id)
}

/// Resolves or reopens a thread, with a note when one is given (as a comment by `by`).
/// Reopening is also how an outdated thread comes back.
pub fn set_status(
    conn: &Connection,
    thread_id: &str,
    status: &str,
    by: &str,
    note: &str,
) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    let review = review_of_thread(&tx, thread_id)?;
    if !note.trim().is_empty() {
        add_comment(&tx, thread_id, by, "comment", note.trim())?;
    }
    tx.execute(
        "UPDATE threads SET status = ?2 WHERE id = ?1",
        params![thread_id, status],
    )?;
    touch(&tx, &review)?;
    tx.commit()?;
    Ok(())
}

/// You opened the thread: its agents' comments so far are read.
pub fn mark_seen(conn: &Connection, thread_id: &str) -> Result<()> {
    conn.execute(
        "UPDATE threads SET last_seen_at = ?2 WHERE id = ?1",
        params![thread_id, now()],
    )?;
    Ok(())
}

/// Where a thread's lines are in `text` (1-based), without storing anything: how a thread made
/// on one version of a file (a commit, or your files then) is shown on another, and how an
/// agent is told whether the code it's about is still there.
pub fn place(conn: &Connection, thread_id: &str, text: &str) -> Result<Option<(u32, u32)>> {
    let row: Option<(u32, u32, String, String, String, Vec<u8>)> = conn
        .query_row(
            "SELECT t.line_start, t.line_end, t.anchor_text, t.context_above, t.context_below,
                    b.content
             FROM threads t JOIN blobs b ON b.hash = t.file_hash WHERE t.id = ?1",
            [thread_id],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            },
        )
        .optional()?;
    let Some((start, end, anchored, above, below, old)) = row else {
        return Ok(None);
    };
    let old = String::from_utf8_lossy(&old);
    let a = crate::reanchor::Anchored {
        old: &old,
        start: start as usize - 1,
        end: end as usize - 1,
        text: &anchored,
        above: &above,
        below: &below,
    };
    Ok(crate::reanchor::place(&a, text).map(|(s, e)| (s as u32 + 1, e as u32 + 1)))
}

/// What [`reanchor`] did to a file's threads.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Reanchored {
    /// Threads placed on new lines.
    pub moved: usize,
    /// Threads whose text is gone, not yet marked outdated: the files haven't settled.
    pub pending: usize,
    pub outdated: usize,
}

/// Places the threads on `path` again now that its content is `new`, and records where they
/// went (with `new` as their content from now on). Threads whose text is gone become outdated
/// only when `settled`, so they don't flicker while an agent is mid-edit. Outdated threads stay
/// where they were until reopened.
pub fn reanchor(
    conn: &Connection,
    review_id: &str,
    path: &str,
    new: &str,
    settled: bool,
) -> Result<Reanchored> {
    /// A thread as stored, with its file's content when it was last placed.
    struct Stored {
        id: String,
        start: u32,
        end: u32,
        text: String,
        above: String,
        below: String,
        status: String,
        old: String,
    }
    let rows: Vec<Stored> = conn
        .prepare(
            "SELECT t.id, t.line_start, t.line_end, t.anchor_text, t.context_above,
                    t.context_below, t.status, b.content
             FROM threads t JOIN blobs b ON b.hash = t.file_hash
             WHERE t.review_id = ?1 AND t.path = ?2 AND t.status != 'outdated'
               AND t.new_rev IS NULL",
        )?
        .query_map(params![review_id, path], |r| {
            let content: Vec<u8> = r.get(7)?;
            Ok(Stored {
                id: r.get(0)?,
                start: r.get(1)?,
                end: r.get(2)?,
                text: r.get(3)?,
                above: r.get(4)?,
                below: r.get(5)?,
                status: r.get(6)?,
                old: String::from_utf8_lossy(&content).into_owned(),
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    let mut done = Reanchored::default();
    let tx = conn.unchecked_transaction()?;
    let mut hash = None;
    for Stored {
        id,
        start,
        end,
        text,
        above,
        below,
        status,
        old,
    } in rows
    {
        if old == new {
            continue;
        }
        let a = crate::reanchor::Anchored {
            old: &old,
            start: start as usize - 1,
            end: end as usize - 1,
            text: &text,
            above: &above,
            below: &below,
        };
        match crate::reanchor::place(&a, new) {
            Some((s, e)) => {
                let hash = match &hash {
                    Some(h) => h,
                    None => hash.insert(put_blob(&tx, new.as_bytes())?),
                };
                let lines: Vec<&str> = new.split_inclusive('\n').collect();
                let ctx = |r: std::ops::Range<usize>| lines.get(r).unwrap_or_default().concat();
                tx.execute(
                    "UPDATE threads SET line_start = ?2, line_end = ?3, file_hash = ?4,
                            context_above = ?5, context_below = ?6
                     WHERE id = ?1",
                    params![
                        id,
                        s as u32 + 1,
                        e as u32 + 1,
                        hash,
                        ctx(s.saturating_sub(3)..s),
                        ctx(e + 1..(e + 4).min(lines.len()))
                    ],
                )?;
                if (s as u32 + 1, e as u32 + 1) != (start, end) {
                    done.moved += 1;
                }
            }
            // A resolved thread whose code is gone stays resolved.
            None if status != "open" => {}
            None if settled => {
                tx.execute(
                    "UPDATE threads SET status = 'outdated' WHERE id = ?1",
                    [&id],
                )?;
                done.outdated += 1;
            }
            None => done.pending += 1,
        }
    }
    tx.commit()?;
    Ok(done)
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

/// What you reviewed in a round: each file's content when you submitted, by path.
pub fn snapshot(
    conn: &Connection,
    review_id: &str,
    round: u32,
) -> Result<std::collections::HashMap<String, Vec<u8>>> {
    Ok(conn
        .prepare(
            "SELECT s.path, b.content FROM snapshot_files s JOIN blobs b ON b.hash = s.hash
             WHERE s.review_id = ?1 AND s.round = ?2",
        )?
        .query_map(params![review_id, round], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?)
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

    fn git(dir: &Path, args: &[&str]) {
        let out = std::process::Command::new("git")
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "-c",
                "init.defaultBranch=main",
            ])
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {out:?}");
    }

    /// A repo on branch `topic` with one commit of its own, and its review.
    fn topic_repo() -> (tempfile::TempDir, Connection, Review) {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q"]);
        std::fs::write(repo.join("a.txt"), "1\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-qm", "one"]);
        git(&repo, &["checkout", "-qb", "topic"]);
        std::fs::write(repo.join("a.txt"), "2\n").unwrap();
        git(&repo, &["commit", "-qam", "two"]);
        let conn = store::open(&dir.path().join("glos.db")).unwrap();
        let r = Review::open(&conn, &Source::git(&repo, None).unwrap()).unwrap();
        (dir, conn, r)
    }

    fn status(conn: &Connection, r: &Review) -> String {
        r.reload(conn).unwrap().status
    }

    #[test]
    fn closing_needs_everything_sent_and_opening_again_reopens() {
        let f = fx();
        review_comment(&f, "unsent");
        let err = close(&f.conn, &f.review.id).unwrap_err().to_string();
        assert!(err.contains("1 comment not sent"), "{err}");
        submit(&f.conn, &f.review.id, Verdict::Comment, &[], None).unwrap();
        close(&f.conn, &f.review.id).unwrap();
        assert_eq!(status(&f.conn, &f.review), "closed");
        assert!(Review::list(&f.conn, false).unwrap().is_empty());
        assert_eq!(Review::list(&f.conn, true).unwrap().len(), 1);
        // Reviewing the same files again opens the same review.
        let again = Review::open(&f.conn, &f.review.source().unwrap()).unwrap();
        assert_eq!(
            (again.id.as_str(), again.status.as_str()),
            (f.review.id.as_str(), "open")
        );
    }

    fn review_comment(f: &Fx, body: &str) {
        comment(&f.conn, &f.review.id, None, YOU, body).unwrap();
    }

    #[test]
    fn reviews_whose_folder_is_gone_are_archived() {
        let f = fx();
        let (_o, n) = (f.dir.path().join("x"), f.dir.path().join("x/new"));
        std::fs::create_dir_all(&n).unwrap();
        std::fs::create_dir_all(f.dir.path().join("x/old")).unwrap();
        std::fs::write(n.join("a"), "1\n").unwrap();
        let gone = Review::open(
            &f.conn,
            &Source::diff(&f.dir.path().join("x/old"), &n).unwrap(),
        )
        .unwrap();
        assert!(archive_finished(&f.conn).unwrap().is_empty());
        std::fs::remove_dir_all(f.dir.path().join("x")).unwrap();
        let done = archive_finished(&f.conn).unwrap();
        assert_eq!(done.iter().map(|r| &r.id).collect::<Vec<_>>(), [&gone.id]);
        assert_eq!(status(&f.conn, &gone), "archived");
        assert_eq!(status(&f.conn, &f.review), "open");
        assert!(archive_finished(&f.conn).unwrap().is_empty(), "once");
    }

    #[test]
    fn a_deleted_branch_archives_its_review() {
        let (dir, conn, r) = topic_repo();
        let repo = dir.path().join("repo");
        assert!(archive_finished(&conn).unwrap().is_empty());
        git(&repo, &["checkout", "-q", "main"]);
        assert!(
            archive_finished(&conn).unwrap().is_empty(),
            "not merged: still to review"
        );
        git(&repo, &["branch", "-qD", "topic"]);
        assert_eq!(archive_finished(&conn).unwrap().len(), 1);
        assert_eq!(status(&conn, &r), "archived");
    }

    #[test]
    fn a_merged_branch_archives_its_review_once_reviewed() {
        let (dir, conn, r) = topic_repo();
        let repo = dir.path().join("repo");
        git(&repo, &["checkout", "-q", "main"]);
        git(&repo, &["merge", "-q", "--ff-only", "topic"]);
        // Merged, but never reviewed: kept, as a fresh branch with no commits looks the same.
        assert!(archive_finished(&conn).unwrap().is_empty());
        submit(&conn, &r.id, Verdict::Approved, &[], None).unwrap();
        assert_eq!(archive_finished(&conn).unwrap().len(), 1);
        assert_eq!(status(&conn, &r), "archived");
    }

    #[test]
    fn a_fresh_branch_with_work_on_disk_stays_open() {
        let (dir, conn, r) = topic_repo();
        let repo = dir.path().join("repo");
        // A new branch at main, after a round: its tip is in main, but there's work to review.
        git(&repo, &["checkout", "-q", "main"]);
        git(&repo, &["checkout", "-qb", "fresh"]);
        std::fs::write(repo.join("b.txt"), "new\n").unwrap();
        let fresh = Review::open(&conn, &Source::git(&repo, None).unwrap()).unwrap();
        submit(&conn, &fresh.id, Verdict::Approved, &[], None).unwrap();
        assert!(archive_finished(&conn).unwrap().is_empty());
        assert_eq!(status(&conn, &fresh), "open");
        assert_eq!(status(&conn, &r), "open");
    }

    #[test]
    fn a_git_review_opens_only_on_its_branch() {
        let (dir, _conn, r) = topic_repo();
        assert!(r.source().is_ok());
        git(&dir.path().join("repo"), &["checkout", "-q", "main"]);
        let err = r.source().err().unwrap().to_string();
        assert!(
            err.contains("is of branch topic") && err.contains("on main"),
            "{err}"
        );
    }

    #[test]
    fn gc_deletes_old_archived_reviews_and_unused_content() {
        let f = fx();
        let blobs = |c: &Connection| -> i64 {
            c.query_row("SELECT count(*) FROM blobs", [], |r| r.get(0))
                .unwrap()
        };
        comment(
            &f.conn,
            &f.review.id,
            Some((&at("new.rs", 2, 2), FILE)),
            YOU,
            "x",
        )
        .unwrap();
        submit(
            &f.conn,
            &f.review.id,
            Verdict::Comment,
            &[("new.rs".into(), b"snap".to_vec())],
            None,
        )
        .unwrap();
        assert_eq!(blobs(&f.conn), 2, "the anchor's file and the snapshot");
        let day = jiff::SignedDuration::from_hours(24);
        // Open: kept, whatever its age.
        assert_eq!(gc(&f.conn, jiff::SignedDuration::ZERO).unwrap(), (0, 0));
        set_review_status(&f.conn, &f.review.id, "archived").unwrap();
        assert_eq!(gc(&f.conn, day).unwrap(), (0, 0), "archived just now");
        // An unused content goes even when no review does.
        put_blob(&f.conn, b"orphan").unwrap();
        assert_eq!(gc(&f.conn, day).unwrap(), (0, 1));
        assert_eq!(gc(&f.conn, jiff::SignedDuration::ZERO).unwrap(), (1, 2));
        assert_eq!(blobs(&f.conn), 0);
        let left: i64 = f
            .conn
            .query_row("SELECT count(*) FROM threads", [], |r| r.get(0))
            .unwrap();
        assert_eq!(left, 0, "its threads went with it");
    }

    #[test]
    fn viewers_are_forgotten_when_their_process_is_gone() {
        let f = fx();
        assert!(!viewing(&f.conn, &f.review.id).unwrap());
        set_viewer(&f.conn, Some(&f.review.id)).unwrap();
        assert!(viewing(&f.conn, &f.review.id).unwrap());
        f.conn
            .execute(
                "UPDATE viewers SET pid = 999999999 WHERE pid = ?1",
                [std::process::id()],
            )
            .unwrap();
        assert!(!viewing(&f.conn, &f.review.id).unwrap());
        let n: i64 = f
            .conn
            .query_row("SELECT count(*) FROM viewers", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
    }

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
        assert_eq!(Review::list(&f.conn, false).unwrap().len(), 1);
    }

    #[test]
    fn reviews_are_found_by_short_id() {
        let f = fx();
        let short = &f.review.id[..4];
        assert_eq!(Review::get(&f.conn, short).unwrap(), Some(f.review.clone()));
        assert_eq!(Review::get(&f.conn, "zzzz").unwrap(), None);
        // A second review is told apart; an empty id names none, even with one review.
        assert!(Review::get(&f.conn, "").is_err());
        let (o, n) = (f.dir.path().join("x"), f.dir.path().join("y"));
        std::fs::write(&o, "1").unwrap();
        std::fs::write(&n, "2").unwrap();
        Review::open(&f.conn, &Source::diff(&o, &n).unwrap()).unwrap();
        assert!(
            Review::get(&f.conn, "")
                .unwrap_err()
                .to_string()
                .contains("no review id")
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
        let snap = snapshot(&f.conn, id, 2).unwrap();
        assert_eq!(snap.len(), 2);
        assert_eq!(snap["b"], b"1");
        assert!(snapshot(&f.conn, id, 9).unwrap().is_empty());
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
    fn agents_annotate_reply_resolve_and_reopen() {
        let f = fx();
        let id = &f.review.id;
        let t = annotate(
            &f.conn,
            id,
            &at("new.rs", 2, 3),
            FILE,
            "claude",
            "why I did this",
        )
        .unwrap();
        let th = |f: &Fx| {
            threads(&f.conn, id)
                .unwrap()
                .into_iter()
                .find(|x| x.id == t)
                .unwrap()
        };
        assert_eq!(th(&f).comments[0].kind, "annotation");
        assert!(
            th(&f).unread(),
            "an agent's note is unread until you open it"
        );
        // Out of range is refused.
        let err = annotate(&f.conn, id, &at("new.rs", 8, 12), FILE, "claude", "x").unwrap_err();
        assert!(err.to_string().contains("8 lines"), "{err}");
        assert!(annotate(&f.conn, id, &at("new.rs", 0, 1), FILE, "claude", "x").is_err());

        mark_seen(&f.conn, &t).unwrap();
        assert!(!th(&f).unread());
        std::thread::sleep(std::time::Duration::from_millis(5));
        reply(&f.conn, &t, "claude", "also").unwrap();
        assert!(th(&f).unread(), "a new reply is unread");
        // Your own replies never are.
        mark_seen(&f.conn, &t).unwrap();
        reply(&f.conn, &t, YOU, "ok").unwrap();
        assert!(!th(&f).unread());

        set_status(
            &f.conn,
            &t,
            "resolved",
            "claude",
            "fixed in the next commit",
        )
        .unwrap();
        assert_eq!(th(&f).status, "resolved");
        assert_eq!(
            th(&f).comments.last().unwrap().body,
            "fixed in the next commit"
        );
        // No note, no comment.
        let n = th(&f).comments.len();
        set_status(&f.conn, &t, "open", YOU, "  ").unwrap();
        assert_eq!((th(&f).status.as_str(), th(&f).comments.len()), ("open", n));
        assert!(set_status(&f.conn, "nope", "open", YOU, "").is_err());
    }

    #[test]
    fn suggestions_are_kept_on_the_review() {
        let f = fx();
        assert_eq!(f.review.suggestion, None);
        f.review
            .suggest(&f.conn, "claude", "new.rs", 4, "start here")
            .unwrap();
        let s = f.review.reload(&f.conn).unwrap().suggestion.unwrap();
        assert_eq!(
            (s.by.as_str(), s.path.as_str(), s.line, s.note.as_str()),
            ("claude", "new.rs", 4, "start here")
        );
    }

    #[test]
    fn reanchoring_moves_threads_and_outdates_them_once_settled() {
        let f = fx();
        let id = &f.review.id;
        let moves = comment(&f.conn, id, Some((&at("new.rs", 5, 6), FILE)), YOU, "e f").unwrap();
        let goes = comment(&f.conn, id, Some((&at("new.rs", 2, 2), FILE)), YOU, "b").unwrap();
        let done = comment(&f.conn, id, Some((&at("new.rs", 3, 3), FILE)), YOU, "c").unwrap();
        set_status(&f.conn, &done, "resolved", YOU, "").unwrap();
        let get = |t: &str| {
            threads(&f.conn, id)
                .unwrap()
                .into_iter()
                .find(|x| x.id == t)
                .unwrap()
        };

        // Two lines added at the top; `b` and `c` rewritten.
        let new = "x\ny\na\nB\nC\nd\ne\nf\ng\nh\n";
        let r = reanchor(&f.conn, id, "new.rs", new, false).unwrap();
        assert_eq!(
            r,
            Reanchored {
                moved: 1,
                pending: 1,
                outdated: 0
            }
        );
        assert_eq!(get(&moves).anchor.unwrap(), at("new.rs", 7, 8));
        assert_eq!(
            get(&goes).status,
            "open",
            "not outdated until the files settle"
        );
        // Once settled it's outdated, and stays where it was.
        let r = reanchor(&f.conn, id, "new.rs", new, true).unwrap();
        assert_eq!(
            r,
            Reanchored {
                moved: 0,
                pending: 0,
                outdated: 1
            }
        );
        assert_eq!(get(&goes).status, "outdated");
        assert_eq!(get(&goes).anchor.unwrap(), at("new.rs", 2, 2));
        // A resolved thread whose code went stays resolved.
        assert_eq!(get(&done).status, "resolved");

        // The moved thread now maps from the new content: another line above moves it again.
        let newer = format!("z\n{new}");
        reanchor(&f.conn, id, "new.rs", &newer, true).unwrap();
        assert_eq!(get(&moves).anchor.unwrap(), at("new.rs", 8, 9));
        // An outdated thread doesn't come back on its own, even if its text returns…
        reanchor(&f.conn, id, "new.rs", FILE, true).unwrap();
        assert_eq!(get(&goes).status, "outdated");
        // …but reopening it brings it back into re-anchoring.
        set_status(&f.conn, &goes, "open", YOU, "").unwrap();
        reanchor(&f.conn, id, "new.rs", FILE, true).unwrap();
        assert_eq!(get(&goes).status, "open");
        // Other files' threads are untouched.
        assert_eq!(
            reanchor(&f.conn, id, "other.rs", "", true).unwrap(),
            Reanchored::default()
        );
    }

    #[test]
    fn threads_keep_the_diff_they_were_made_on() {
        let f = fx();
        let id = &f.review.id;
        let on = DiffContext {
            old_rev: Some("aaa".into()),
            new_rev: Some("bbb".into()),
            diff: Some("commit 1/2 bbb \"x\" vs its parent aaa".into()),
        };
        let t = comment_in(
            &f.conn,
            id,
            Some((&at("new.rs", 2, 2), FILE)),
            YOU,
            "c",
            &on,
        )
        .unwrap();
        let th = threads(&f.conn, id)
            .unwrap()
            .into_iter()
            .find(|x| x.id == t)
            .unwrap();
        assert_eq!(th.on, on);
        // Kept with a review-level thread too, and absent for plain comments.
        let r = comment_in(&f.conn, id, None, YOU, "r", &on).unwrap();
        let th = threads(&f.conn, id)
            .unwrap()
            .into_iter()
            .find(|x| x.id == r)
            .unwrap();
        assert_eq!(th.on.new_rev.as_deref(), Some("bbb"));
        let p = comment(&f.conn, id, Some((&at("new.rs", 3, 3), FILE)), YOU, "p").unwrap();

        // Files change: only the thread on your files moves; the commit's stays on its code.
        let new = format!("top\n{FILE}");
        let r = reanchor(&f.conn, id, "new.rs", &new, true).unwrap();
        assert_eq!(r.moved, 1);
        let lines = |t: &str| {
            let th = threads(&f.conn, id)
                .unwrap()
                .into_iter()
                .find(|x| x.id == t)
                .unwrap();
            th.anchor.map(|a| (a.start, a.end))
        };
        assert_eq!(lines(&p), Some((4, 4)));
        assert_eq!(lines(&t), Some((2, 2)));
        // But it can be placed on the new text, to show it there or tell an agent it's still there.
        assert_eq!(place(&f.conn, &t, &new).unwrap(), Some((3, 3)));
        assert_eq!(place(&f.conn, &t, "gone\n").unwrap(), None);
        assert_eq!(place(&f.conn, "nope", &new).unwrap(), None);
        // As JSON the context is part of the thread.
        let json = serde_json::to_value(threads(&f.conn, id).unwrap()).unwrap();
        assert!(
            json.as_array()
                .unwrap()
                .iter()
                .any(|t| t["new_rev"] == "bbb")
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
