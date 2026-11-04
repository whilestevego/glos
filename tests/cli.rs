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
