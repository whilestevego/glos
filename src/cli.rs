//! `glos list`, `glos show` and `glos submit`: the review state for scripts and for agents
//! without MCP. Text by default; with `--json`, documents that carry a `"version"`, whose shape
//! changes only with it.

use std::{io::Write, path::Path};

use anyhow::{Context, Result};
use rusqlite::Connection;
use serde_json::json;

use crate::review::{self, Review, Verdict};

const VERSION: u32 = 1;

/// The review named by (the start of) its id. glos never picks one for you.
fn find(conn: &Connection, id: &str) -> Result<Review> {
    Review::get(conn, id)?.with_context(|| format!("no review {id} (glos list shows them)"))
}

pub fn list(conn: &Connection, all: bool, json: bool, out: &mut impl Write) -> Result<()> {
    review::archive_finished(conn)?;
    let reviews = Review::list(conn, all)?;
    let mut rows = vec![];
    for r in &reviews {
        let threads = review::threads(conn, &r.id)?;
        let open = threads.iter().filter(|t| t.status == "open").count();
        rows.push((r, review::unsent(conn, &r.id)?, open));
    }
    if json {
        let reviews: Vec<_> = rows
            .iter()
            .map(|(r, unsent, open)| {
                let mut v = serde_json::to_value(r).unwrap();
                v["unsent"] = json!(unsent);
                v["open_threads"] = json!(open);
                v
            })
            .collect();
        writeln!(out, "{}", json!({ "version": VERSION, "reviews": reviews }))?;
        return Ok(());
    }
    for (r, unsent, open) in rows {
        let mut line = format!("{}  {:<6}  {}", r.id, r.status, r.describe());
        if unsent > 0 {
            line += &format!(" · {unsent} unsent");
        }
        if open > 0 {
            line += &format!(" · {open} open thread{}", if open == 1 { "" } else { "s" });
        }
        if r.requested_at.is_some() {
            line += " · review requested";
        }
        writeln!(out, "{line}")?;
    }
    Ok(())
}

pub fn show(conn: &Connection, id: &str, json: bool, out: &mut impl Write) -> Result<()> {
    let r = find(conn, id)?;
    let rounds = review::rounds(conn, &r.id)?;
    let threads = review::threads(conn, &r.id)?;
    if json {
        let doc = json!({ "version": VERSION, "review": r, "rounds": rounds, "threads": threads });
        writeln!(out, "{doc}")?;
        return Ok(());
    }
    writeln!(out, "Review {} of {} ({})", r.id, r.describe(), r.status)?;
    if let (Some(by), Some(at)) = (&r.requested_by, &r.requested_at) {
        let note = r.request_note.as_deref().unwrap_or("");
        writeln!(out, "Review requested by {by} at {at}: {note}")?;
    }
    for round in &rounds {
        let to = round
            .delivered_to
            .as_ref()
            .map_or(" (not delivered yet)".into(), |a| {
                format!(" (delivered to {a})")
            });
        writeln!(
            out,
            "Round {}: {}, {} comment{}{to}",
            round.n,
            round.verdict.describe(),
            round.comments,
            if round.comments == 1 { "" } else { "s" }
        )?;
    }
    for t in &threads {
        let place = t
            .anchor
            .as_ref()
            .map_or("the review".into(), |a| a.to_string());
        writeln!(out, "\nThread {} on {place} ({})", t.id, t.status)?;
        if let Some(d) = &t.on.diff {
            writeln!(out, "  made on: {d}")?;
        }
        for c in &t.comments {
            let mut who = c.author.clone();
            if c.unsent() {
                who += ", unsent";
            }
            if c.edited_at.is_some() {
                who += ", edited";
            }
            writeln!(out, "  {who}:")?;
            for l in c.body.lines() {
                writeln!(out, "    {l}")?;
            }
        }
    }
    Ok(())
}

/// `glos close`: you're done with the review.
pub fn close(conn: &Connection, id: &str, json: bool, out: &mut impl Write) -> Result<()> {
    let r = find(conn, id)?;
    if r.status != "open" {
        anyhow::bail!("review {} is already {}", r.id, r.status);
    }
    review::close(conn, &r.id)?;
    if json {
        writeln!(out, "{}", json!({ "version": VERSION, "closed": r.id }))?;
    } else {
        writeln!(out, "Closed review {} of {}.", r.id, r.describe())?;
    }
    Ok(())
}

/// `glos gc`: archives what's over, then deletes old archived reviews and unused content.
pub fn gc(conn: &Connection, days: u32, json: bool, out: &mut impl Write) -> Result<()> {
    let archived = review::archive_finished(conn)?.len();
    let age = jiff::SignedDuration::from_hours(24 * i64::from(days));
    let (reviews, blobs) = review::gc(conn, age)?;
    if json {
        let doc = json!({ "version": VERSION, "archived": archived, "deleted_reviews": reviews,
                          "deleted_contents": blobs });
        writeln!(out, "{doc}")?;
    } else {
        writeln!(
            out,
            "Archived {archived} finished review(s); deleted {reviews} archived more than \
             {days} days ago, and {blobs} file content(s) nothing used."
        )?;
    }
    Ok(())
}

/// `glos open-review`: the review of exactly what's named, made if there's none yet, as the
/// MCP tool does. Prints its id.
pub fn open_review(
    conn: Connection,
    cwd: &Path,
    target: crate::mcp::Target,
    json: bool,
    out: &mut impl Write,
) -> Result<()> {
    let mut a = crate::mcp::Agent::new(conn, cwd.to_path_buf());
    let (r, files) = a.open_target(&target)?.map_err(anyhow::Error::msg)?;
    if json {
        let doc = json!({ "version": VERSION, "review": r, "changed_files": files });
        writeln!(out, "{doc}")?;
    } else {
        writeln!(out, "{}  {}", r.id, r.describe())?;
    }
    Ok(())
}

/// `glos open`: opens glos on a review the way `request_review` does. `false` when it couldn't
/// (no `open-command`), after saying how to open it by hand.
pub fn open(
    conn: &Connection,
    id: &str,
    open_command: Option<&str>,
    db: &Path,
    out: &mut impl Write,
) -> Result<bool> {
    use crate::launch::{self, Launch};
    let r = find(conn, id)?;
    match launch::launch(conn, &r, open_command, db)? {
        Launch::AlreadyOpen => writeln!(out, "glos is already open on {}.", r.describe())?,
        Launch::Ran(cmd) => writeln!(out, "Opened glos on {}: {cmd}", r.describe())?,
        Launch::NoCommand => {
            eprintln!(
                "No open-command in config.toml, so glos can't open itself. Run {}.",
                launch::by_hand(&r)
            );
            return Ok(false);
        }
    }
    Ok(true)
}

pub fn submit(
    conn: &Connection,
    id: &str,
    verdict: Verdict,
    json: bool,
    out: &mut impl Write,
) -> Result<()> {
    let r = find(conn, id)?;
    // What was reviewed: every file's new side as it is now.
    let source = r.source()?;
    let files: Vec<(String, Vec<u8>)> = source
        .changes
        .iter()
        .filter_map(|c| Some((c.path.clone(), source.read(&c.new).ok()??)))
        .collect();
    let head = source.git.as_ref().map(|g| g.head.to_string());
    let round = review::submit(conn, &r.id, verdict, &files, head.as_deref())?;
    if json {
        writeln!(out, "{}", json!({ "version": VERSION, "round": round }))?;
    } else {
        writeln!(
            out,
            "Round {} submitted: {}, {} comment{}.",
            round.n,
            verdict.describe(),
            round.comments,
            if round.comments == 1 { "" } else { "s" }
        )?;
    }
    Ok(())
}

/// What `glos reply`, `resolve`, `reopen` and `annotate` do: an agent's action, through the
/// same checks as the MCP server's tools.
pub enum AgentAction {
    Reply {
        thread: String,
        body: String,
    },
    Resolve {
        thread: String,
        note: String,
    },
    Reopen {
        thread: String,
        note: String,
    },
    Annotate {
        path: String,
        lines: String,
        body: String,
    },
}

/// Runs an agent action on review `id`. A refusal is an error, so the exit code is 1.
pub fn agent(
    conn: Connection,
    cwd: &Path,
    id: &str,
    author: &str,
    action: AgentAction,
    json: bool,
    out: &mut impl Write,
) -> Result<()> {
    let mut a = crate::mcp::Agent::new(conn, cwd.to_path_buf());
    let outcome = match action {
        AgentAction::Reply { thread, body } => a.reply(author, id, &thread, &body)?,
        AgentAction::Resolve { thread, note } => a.set_status(author, id, &thread, false, &note)?,
        AgentAction::Reopen { thread, note } => a.set_status(author, id, &thread, true, &note)?,
        AgentAction::Annotate { path, lines, body } => {
            let (start, end) = match lines.split_once('-') {
                Some((s, e)) => (s.parse(), e.parse().map(Some)),
                None => (lines.parse(), Ok(None)),
            };
            let (Ok(start), Ok(end)) = (start, end) else {
                anyhow::bail!("lines are `12` or `12-14`, not `{lines}`");
            };
            a.annotate(author, id, &path, start, end, &body)?
        }
    };
    let message = outcome.map_err(anyhow::Error::msg)?;
    if json {
        writeln!(out, "{}", json!({ "version": VERSION, "message": message }))?;
    } else {
        writeln!(out, "{message}")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{review::YOU, source::Source};

    fn setup() -> (tempfile::TempDir, Connection, Review, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let (o, n) = (dir.path().join("o"), dir.path().join("n"));
        std::fs::create_dir_all(&o).unwrap();
        std::fs::create_dir_all(&n).unwrap();
        std::fs::write(n.join("a.rs"), "new\n").unwrap();
        let conn = crate::store::open(&dir.path().join("glos.db")).unwrap();
        let r = Review::open(&conn, &Source::diff(&o, &n).unwrap()).unwrap();
        let cwd = n.canonicalize().unwrap();
        (dir, conn, r, cwd)
    }

    fn run(f: impl FnOnce(&mut Vec<u8>) -> Result<()>) -> String {
        let mut out = vec![];
        f(&mut out).unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn list_and_show_as_text_and_json() {
        let (_d, conn, r, _cwd) = setup();
        review::comment(&conn, &r.id, None, YOU, "looks\ngood").unwrap();
        let text = run(|o| list(&conn, false, false, o));
        assert!(text.starts_with(&r.id), "{text}");
        assert!(text.contains("· 1 unsent · 1 open thread"), "{text}");
        let doc: serde_json::Value =
            serde_json::from_str(&run(|o| list(&conn, false, true, o))).unwrap();
        assert_eq!(doc["version"], 1);
        assert_eq!(doc["reviews"][0]["id"], r.id.as_str());
        assert_eq!(doc["reviews"][0]["unsent"], 1);

        // `show` takes the review's id, or the start of it.
        let text = run(|o| show(&conn, &r.id, false, o));
        assert!(
            text.contains("  you, unsent:\n    looks\n    good"),
            "{text}"
        );
        let again = run(|o| show(&conn, &r.id[..5], false, o));
        assert_eq!(text, again);
        let doc: serde_json::Value =
            serde_json::from_str(&run(|o| show(&conn, &r.id, true, o))).unwrap();
        assert_eq!(doc["threads"][0]["comments"][0]["body"], "looks\ngood");
        assert_eq!(doc["rounds"], json!([]));
    }

    #[test]
    fn a_review_is_always_named() {
        let (_d, conn, _r, _cwd) = setup();
        let mut out = vec![];
        for id in ["nope", ""] {
            assert!(show(&conn, id, false, &mut out).is_err(), "{id:?}");
            assert!(close(&conn, id, false, &mut out).is_err(), "{id:?}");
            assert!(submit(&conn, id, Verdict::Approved, false, &mut out).is_err());
        }
        let err = show(&conn, "nope", false, &mut out)
            .unwrap_err()
            .to_string();
        assert!(err.contains("glos list"), "{err}");
    }

    #[test]
    fn open_review_prints_the_id() {
        let (d, conn, r, cwd) = setup();
        let target = crate::mcp::Target::Diff {
            old: d.path().join("o").to_string_lossy().into(),
            new: d.path().join("n").to_string_lossy().into(),
        };
        let db = conn.path().unwrap().to_string();
        let c = || crate::store::open(Path::new(&db)).unwrap();
        let text = run(|o| open_review(c(), &cwd, target.clone(), false, o));
        assert!(text.starts_with(&format!("{}  ", r.id)), "{text}");
        let doc: serde_json::Value =
            serde_json::from_str(&run(|o| open_review(c(), &cwd, target, true, o))).unwrap();
        assert_eq!(doc["review"]["id"], r.id.as_str());
        assert_eq!(doc["changed_files"], 1);
        let bad = crate::mcp::Target::Diff {
            old: "/no/such".into(),
            new: "/no/such/either".into(),
        };
        assert!(open_review(c(), &cwd, bad, false, &mut vec![]).is_err());
    }

    #[test]
    fn empty_results_are_not_errors() {
        let dir = tempfile::tempdir().unwrap();
        let conn = crate::store::open(&dir.path().join("glos.db")).unwrap();
        assert_eq!(run(|o| list(&conn, true, false, o)), "");
        assert_eq!(
            run(|o| list(&conn, false, true, o)),
            "{\"reviews\":[],\"version\":1}\n"
        );
    }

    #[test]
    fn agent_actions_from_the_cli() {
        let (_d, conn, r, cwd) = setup();
        let db = conn.path().unwrap().to_string();
        let c = || crate::store::open(Path::new(&db)).unwrap();
        let run_agent = |action| {
            let mut out = vec![];
            agent(c(), &cwd, &r.id, "script", action, false, &mut out)
                .map(|_| String::from_utf8(out).unwrap())
        };
        let note = run_agent(AgentAction::Annotate {
            path: "a.rs".into(),
            lines: "1".into(),
            body: "new file".into(),
        })
        .unwrap();
        assert!(note.starts_with("Note left on a.rs:1"), "{note}");
        let t = review::threads(&conn, &r.id).unwrap()[0].id.clone();
        assert_eq!(
            run_agent(AgentAction::Reply {
                thread: t.clone(),
                body: "more".into()
            })
            .unwrap(),
            format!("Replied on thread {t}.\n")
        );
        // A refusal is an error, for the exit code.
        let err = run_agent(AgentAction::Resolve {
            thread: t.clone(),
            note: "".into(),
        })
        .unwrap_err();
        assert!(err.to_string().contains("needs a note"));
        run_agent(AgentAction::Resolve {
            thread: t.clone(),
            note: "done".into(),
        })
        .unwrap();
        run_agent(AgentAction::Reopen {
            thread: t.clone(),
            note: "".into(),
        })
        .unwrap();
        let bad = run_agent(AgentAction::Annotate {
            path: "a.rs".into(),
            lines: "x-2".into(),
            body: "b".into(),
        });
        assert!(bad.unwrap_err().to_string().contains("12-14"));
        let th = &review::threads(&conn, &r.id).unwrap()[0];
        assert_eq!(th.status, "open");
        assert_eq!(
            th.comments
                .iter()
                .map(|c| c.author.as_str())
                .collect::<Vec<_>>(),
            ["script"; 3]
        );
    }

    #[test]
    fn submit_from_the_cli() {
        let (_d, conn, r, _cwd) = setup();
        let err = submit(&conn, &r.id, Verdict::Changes, false, &mut vec![]).unwrap_err();
        assert!(err.to_string().contains("nothing to submit"));
        review::comment(&conn, &r.id, None, YOU, "x").unwrap();
        let text = run(|o| submit(&conn, &r.id, Verdict::Changes, false, o));
        assert_eq!(text, "Round 1 submitted: changes requested, 1 comment.\n");
        let doc: serde_json::Value =
            serde_json::from_str(&run(|o| submit(&conn, &r.id, Verdict::Approved, true, o)))
                .unwrap();
        assert_eq!(doc["round"]["n"], 2);
        assert_eq!(doc["round"]["verdict"], "approved");
        let text = run(|o| show(&conn, &r.id, false, o));
        assert!(text.contains("Round 1: changes requested, 1 comment (not delivered yet)"));
        // The snapshot holds the reviewed file.
        let n: i64 = conn
            .query_row(
                "SELECT count(*) FROM snapshot_files WHERE round = 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1);
        assert!(submit(&conn, "nope", Verdict::Approved, false, &mut vec![]).is_err());
    }
}
