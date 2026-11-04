//! The MCP server end to end, as an agent host drives it: `glos mcp` speaking JSON-RPC over
//! stdio in a git repo, with the user's submit made through the CLI in between.

use std::{
    io::{BufRead, BufReader, Write},
    path::Path,
    process::{Command, Stdio},
};

use serde_json::{Value, json};

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(["-c", "init.defaultBranch=main"])
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {out:?}");
}

/// `glos` with the test's own database and log folder.
fn glos(dir: &Path, repo: &Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_glos"));
    c.current_dir(repo)
        .env("GLOS_DB", dir.join("glos.db"))
        .env("XDG_STATE_HOME", dir.join("state"));
    c
}

#[test]
fn request_submit_and_wait() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    std::fs::write(repo.join("a.rs"), "fn a() {}\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "one"]);
    git(&repo, &["checkout", "-qb", "topic"]);
    std::fs::write(repo.join("a.rs"), "fn a() { todo!() }\n").unwrap();

    let mut server = glos(dir.path(), &repo)
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = server.stdin.take().unwrap();
    let mut stdout = BufReader::new(server.stdout.take().unwrap());
    let mut send = |msg: Value| writeln!(stdin, "{msg}").unwrap();
    let mut recv = |id: u64| -> Value {
        loop {
            let mut line = String::new();
            assert!(
                stdout.read_line(&mut line).unwrap() > 0,
                "the server closed"
            );
            let v: Value = serde_json::from_str(&line).unwrap();
            if v["id"] == id {
                return v;
            }
        }
    };
    let mut id = 0;
    let mut call = |method: &str, params: Value| -> Value {
        id += 1;
        send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        if method == "initialize" {
            let v = recv(id);
            // The handshake ends with a notification, which gets no answer.
            send(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }));
            return v;
        }
        recv(id)
    };
    let init = call(
        "initialize",
        json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": { "name": "test-agent", "version": "1" }
        }),
    );
    assert!(
        init["result"]["instructions"]
            .as_str()
            .unwrap()
            .contains("request_review")
    );
    let tools = call("tools/list", json!({}));
    let mut names: Vec<&str> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    names.sort();
    assert_eq!(
        names,
        [
            "annotate",
            "get_review",
            "reopen",
            "reply",
            "request_review",
            "resolve",
            "suggest",
            "wait_for_review"
        ]
    );

    let text = |v: &Value| {
        v["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let r = call(
        "tools/call",
        json!({ "name": "request_review", "arguments": { "note": "added todo" } }),
    );
    assert!(
        text(&r).contains("Review requested for branch topic (vs main)"),
        "{r}"
    );

    // Nothing submitted: a short wait says so.
    let r = call(
        "tools/call",
        json!({ "name": "wait_for_review", "arguments": { "timeout_secs": 1 } }),
    );
    assert!(text(&r).contains("hasn't submitted"), "{r}");

    // The user approves from the CLI; the waiting agent gets it.
    let out = glos(dir.path(), &repo)
        .args(["submit", "approve"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "Round 1 submitted: approved, 0 comments.\n"
    );
    let r = call(
        "tools/call",
        json!({ "name": "wait_for_review", "arguments": { "timeout_secs": 10 } }),
    );
    assert!(text(&r).contains("(round 1): approved"), "{r}");

    let r = call(
        "tools/call",
        json!({ "name": "get_review", "arguments": {} }),
    );
    assert!(text(&r).contains("- a.rs (Modified)"), "{r}");

    // The CLI shows who received it.
    let out = glos(dir.path(), &repo)
        .args(["show", "--json"])
        .output()
        .unwrap();
    let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(doc["version"], 1);
    assert_eq!(doc["rounds"][0]["delivered_to"], "test-agent", "{doc}");
    // The server registered itself under the agent's name while it ran.
    let out = glos(dir.path(), &repo).arg("doctor").output().unwrap();
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("MCP: 1 agent session attached"),
        "{out:?}"
    );
    // Closing stdin ends the server cleanly, and it removes its session.
    drop(stdin);
    assert!(server.wait().unwrap().success());
    let out = glos(dir.path(), &repo).arg("doctor").output().unwrap();
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("MCP: 0 agent sessions"),
        "{out:?}"
    );
}
