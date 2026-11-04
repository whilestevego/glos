//! `glos list`, `glos show` and `glos submit`: the review state for scripts and for agents
//! without MCP. Text by default; with `--json`, documents that carry a `"version"`, whose shape
//! changes only with it.

use std::{io::Write, path::Path};

use anyhow::{Context, Result};
use rusqlite::Connection;
use serde_json::json;

use crate::review::{self, Review, Verdict};

const VERSION: u32 = 1;

/// The review named by (the start of) its id, or the one for this folder.
fn find(conn: &Connection, id: Option<&str>, cwd: &Path) -> Result<Option<Review>> {
    match id {
        Some(id) => Review::get(conn, id),
        None => Review::for_folder(conn, cwd),
    }
}

pub fn list(conn: &Connection, all: bool, json: bool, out: &mut impl Write) -> Result<()> {
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

pub fn show(
    conn: &Connection,
    id: Option<&str>,
    cwd: &Path,
    json: bool,
    out: &mut impl Write,
) -> Result<()> {
    let Some(r) = find(conn, id, cwd)? else {
        if json {
            writeln!(out, "{}", json!({ "version": VERSION, "review": null }))?;
        } else {
            writeln!(out, "No review here.")?;
        }
        return Ok(());
    };
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

pub fn submit(
    conn: &Connection,
    id: Option<&str>,
    cwd: &Path,
    verdict: Verdict,
    json: bool,
    out: &mut impl Write,
) -> Result<()> {
    let r = find(conn, id, cwd)?.context("no review here; give its id (see glos list)")?;
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
        let (_d, conn, r, cwd) = setup();
        review::comment(&conn, &r.id, None, YOU, "looks\ngood").unwrap();
        let text = run(|o| list(&conn, false, false, o));
        assert!(text.starts_with(&r.id), "{text}");
        assert!(text.contains("· 1 unsent · 1 open thread"), "{text}");
        let doc: serde_json::Value =
            serde_json::from_str(&run(|o| list(&conn, false, true, o))).unwrap();
        assert_eq!(doc["version"], 1);
        assert_eq!(doc["reviews"][0]["id"], r.id.as_str());
        assert_eq!(doc["reviews"][0]["unsent"], 1);

        // `show` finds the review from the folder it runs in, or by a short id.
        let text = run(|o| show(&conn, None, &cwd, false, o));
        assert!(
            text.contains("  you, unsent:\n    looks\n    good"),
            "{text}"
        );
        let again = run(|o| show(&conn, Some(&r.id[..5]), Path::new("/"), false, o));
        assert_eq!(text, again);
        let doc: serde_json::Value =
            serde_json::from_str(&run(|o| show(&conn, None, &cwd, true, o))).unwrap();
        assert_eq!(doc["threads"][0]["comments"][0]["body"], "looks\ngood");
        assert_eq!(doc["rounds"], json!([]));
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
        assert_eq!(
            run(|o| show(&conn, None, dir.path(), false, o)),
            "No review here.\n"
        );
        let doc: serde_json::Value =
            serde_json::from_str(&run(|o| show(&conn, None, dir.path(), true, o))).unwrap();
        assert!(doc["review"].is_null());
    }

    #[test]
    fn submit_from_the_cli() {
        let (_d, conn, r, cwd) = setup();
        let err = submit(&conn, None, &cwd, Verdict::Changes, false, &mut vec![]).unwrap_err();
        assert!(err.to_string().contains("nothing to submit"));
        review::comment(&conn, &r.id, None, YOU, "x").unwrap();
        let text = run(|o| submit(&conn, None, &cwd, Verdict::Changes, false, o));
        assert_eq!(text, "Round 1 submitted: changes requested, 1 comment.\n");
        let doc: serde_json::Value = serde_json::from_str(&run(|o| {
            submit(&conn, Some(&r.id), &cwd, Verdict::Approved, true, o)
        }))
        .unwrap();
        assert_eq!(doc["round"]["n"], 2);
        assert_eq!(doc["round"]["verdict"], "approved");
        let text = run(|o| show(&conn, None, &cwd, false, o));
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
        assert!(
            submit(
                &conn,
                None,
                Path::new("/"),
                Verdict::Approved,
                false,
                &mut vec![]
            )
            .is_err()
        );
    }
}
