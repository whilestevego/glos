//! `glos diff` from the shell: how it takes files, folders and revisions, without the UI.

use std::{path::Path, process::Command};

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
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

fn glos(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_glos"))
        .args(args)
        .current_dir(dir.join("repo"))
        .env("GLOS_DB", dir.join("glos.db"))
        .env("XDG_STATE_HOME", dir.join("state"))
        .env("XDG_CONFIG_HOME", dir.join("config"))
        .output()
        .unwrap()
}

#[test]
fn diff_takes_revisions() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path().join("repo");
    std::fs::create_dir_all(&r).unwrap();
    git(&r, &["init", "-q"]);
    std::fs::write(r.join("a.txt"), "1\n").unwrap();
    git(&r, &["add", "."]);
    git(&r, &["commit", "-qm", "one"]);

    let out = glos(d.path(), &["diff", "HEAD", "HEAD"]);
    assert!(out.status.success(), "{out:?}");
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "Nothing to review: no changes between HEAD and HEAD.\n"
    );
    let out = glos(d.path(), &["diff", "HEAD"]);
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "Nothing to review: no changes between HEAD and your files.\n"
    );
    // Not a file, folder or revision: an error, saying so.
    let out = glos(d.path(), &["diff", "nope"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("`nope` is neither a file or folder"));
    // One path alone isn't a comparison: bad usage.
    let out = glos(d.path(), &["diff", "a.txt"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("or a git revision"));
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

#[test]
fn completions_and_man_page() {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(d.path().join("repo")).unwrap();
    for shell in ["bash", "zsh", "fish"] {
        let out = glos(d.path(), &["completions", shell]);
        assert!(out.status.success(), "{out:?}");
        assert!(text(&out.stdout).contains("dashboard"), "{shell}");
    }
    assert_eq!(
        glos(d.path(), &["completions", "nope"]).status.code(),
        Some(2)
    );
    let out = glos(d.path(), &["man"]);
    assert!(out.status.success());
    let man = text(&out.stdout);
    assert!(man.starts_with(".ie") || man.contains(".TH glos"), "{man}");
    assert!(man.contains("dashboard") && man.contains("gc"));
}

#[test]
fn a_review_from_open_to_close_to_gc() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path().join("repo");
    std::fs::create_dir_all(&r).unwrap();
    git(&r, &["init", "-q"]);
    std::fs::write(r.join("a.txt"), "1\n").unwrap();
    git(&r, &["add", "."]);
    git(&r, &["commit", "-qm", "one"]);
    git(&r, &["checkout", "-qb", "topic"]);
    std::fs::write(r.join("a.txt"), "2\n").unwrap();

    // Every command names its review: none is found from the folder.
    let out = glos(d.path(), &["show"]);
    assert_eq!(out.status.code(), Some(2), "--review is required: {out:?}");
    // The review is made from exactly what's named, and its id printed.
    let out = glos(d.path(), &["open-review", "git", ".", "main", "main"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        text(&out.stderr).contains("has topic checked out, not main"),
        "{out:?}"
    );
    let out = glos(
        d.path(),
        &["--json", "open-review", "git", ".", "topic", "main"],
    );
    assert!(out.status.success(), "{out:?}");
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let id = doc["review"]["id"].as_str().unwrap().to_string();
    assert_eq!(doc["review"]["branch"], "topic");

    // No open-command: glos says how to open it by hand, and fails so scripts can tell.
    let out = glos(d.path(), &["open", "--review", &id]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert!(text(&out.stderr).contains("No open-command"), "{out:?}");
    // With one, glos runs it with its own command line.
    let log = d.path().join("opened");
    std::fs::create_dir_all(d.path().join("config/glos")).unwrap();
    std::fs::write(
        d.path().join("config/glos/config.toml"),
        format!("open-command = \"echo >> '{}'\"\n", log.display()),
    )
    .unwrap();
    let out = glos(d.path(), &["open", "--review", &id]);
    assert!(out.status.success(), "{out:?}");
    assert!(
        text(&out.stdout).starts_with("Opened glos on branch topic"),
        "{out:?}"
    );
    let opened = std::fs::read_to_string(&log).unwrap();
    assert!(
        opened.contains(" --db ") && opened.contains(" --review "),
        "{opened}"
    );

    let out = glos(d.path(), &["--json", "list"]);
    let list: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(list["reviews"][0]["id"], id.as_str());

    let out = glos(d.path(), &["--review", "nope"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(text(&out.stderr).contains("no review nope"), "{out:?}");

    let out = glos(d.path(), &["close", "--review", &id]);
    assert!(out.status.success(), "{out:?}");
    assert!(text(&out.stdout).starts_with(&format!("Closed review {id}")));
    assert_eq!(text(&glos(d.path(), &["list"]).stdout), "");
    assert!(text(&glos(d.path(), &["list", "--all"]).stdout).contains("closed"));
    let out = glos(d.path(), &["close", "--review", &id]);
    assert_eq!(out.status.code(), Some(1), "already closed: {out:?}");
    let out = glos(d.path(), &["dashboard"]);
    assert!(out.status.success());
    assert!(text(&out.stdout).starts_with("No open reviews"), "{out:?}");

    // Deleting the branch archives the review; gc deletes it once it's old enough.
    git(&r, &["checkout", "-q", "main"]);
    git(&r, &["branch", "-qD", "topic"]);
    let out = glos(d.path(), &["gc"]);
    assert!(
        text(&out.stdout).starts_with("Archived 1 finished review(s); deleted 0"),
        "{out:?}"
    );
    let out = glos(d.path(), &["--json", "gc", "--days", "0"]);
    let gc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(gc["deleted_reviews"], 1, "{gc}");
    assert_eq!(gc["version"], 1);
    assert_eq!(text(&glos(d.path(), &["list", "--all"]).stdout), "");
}
