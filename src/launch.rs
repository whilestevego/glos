//! Opening glos for a review, as `request_review` and `glos open` do: it's already open, or
//! `open-command` opens it (a tmux, kitty or wezterm split…), or nothing can.

use std::path::Path;

use anyhow::{Result, bail};
use rusqlite::Connection;

use crate::review::{self, Review};

#[derive(Debug, PartialEq)]
pub enum Launch {
    /// A running glos shows the review.
    AlreadyOpen,
    /// `open-command` ran this.
    Ran(String),
    /// No `open-command` is set.
    NoCommand,
}

/// Opens glos on `r` unless one already shows it. `open_command` is the start of a shell
/// command that runs the rest in a new pane; glos adds itself, the database and the review.
pub fn launch(
    conn: &Connection,
    r: &Review,
    open_command: Option<&str>,
    db: &Path,
) -> Result<Launch> {
    if review::viewing(conn, &r.id)? {
        return Ok(Launch::AlreadyOpen);
    }
    let Some(prefix) = open_command.filter(|c| !c.trim().is_empty()) else {
        return Ok(Launch::NoCommand);
    };
    let exe = std::env::current_exe()?;
    let cmd = format!(
        "{prefix} {} --db {} --review {}",
        quote(&exe.to_string_lossy()),
        quote(&db.to_string_lossy()),
        r.id
    );
    // Nothing may reach our stdout: in `glos mcp` it carries the protocol.
    let out = std::process::Command::new("sh")
        .arg("-c")
        .arg(&cmd)
        .current_dir(&r.folder)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .output()?;
    if !out.status.success() {
        bail!(
            "open-command failed ({}): {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(Launch::Ran(cmd))
}

/// The way to open the review by hand.
pub fn by_hand(r: &Review) -> String {
    if r.kind == "git" {
        format!("`glos` in {}", r.folder)
    } else {
        format!("`glos --review {}`", r.id)
    }
}

/// Single-quoted for `sh`.
fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{source::Source, store};

    #[test]
    fn open_command_runs_once_glos_is_not_open() {
        let dir = tempfile::tempdir().unwrap();
        let (o, n) = (dir.path().join("o.rs"), dir.path().join("it's.rs"));
        std::fs::write(&o, "a\n").unwrap();
        std::fs::write(&n, "b\n").unwrap();
        let db = dir.path().join("glos.db");
        let conn = store::open(&db).unwrap();
        let r = Review::open(&conn, &Source::diff(&o, &n).unwrap()).unwrap();
        assert_eq!(launch(&conn, &r, None, &db).unwrap(), Launch::NoCommand);
        assert_eq!(
            launch(&conn, &r, Some("  "), &db).unwrap(),
            Launch::NoCommand
        );
        assert!(by_hand(&r).contains(&r.id));

        let log = dir.path().join("log");
        let prefix = format!("printf '%s\\n' >> {}", quote(&log.to_string_lossy()));
        let Launch::Ran(cmd) = launch(&conn, &r, Some(&prefix), &db).unwrap() else {
            panic!()
        };
        assert!(cmd.ends_with(&format!("--review {}", r.id)), "{cmd}");
        let args = std::fs::read_to_string(&log).unwrap();
        let args: Vec<&str> = args.lines().collect();
        assert_eq!(
            args[1..],
            ["--db", &*db.to_string_lossy(), "--review", &r.id]
        );
        assert!(
            args[0].ends_with(
                std::env::current_exe()
                    .unwrap()
                    .file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
            )
        );

        let err = launch(&conn, &r, Some("false"), &db).unwrap_err();
        assert!(err.to_string().contains("open-command failed"), "{err}");

        // A glos showing it: nothing runs.
        review::set_viewer(&conn, Some(&r.id)).unwrap();
        assert_eq!(
            launch(&conn, &r, Some(&prefix), &db).unwrap(),
            Launch::AlreadyOpen
        );
        review::set_viewer(&conn, None).unwrap();
        assert!(!review::viewing(&conn, &r.id).unwrap());
    }

    #[test]
    fn quotes_for_sh() {
        assert_eq!(quote("a b"), "'a b'");
        assert_eq!(quote("it's"), r"'it'\''s'");
    }
}
