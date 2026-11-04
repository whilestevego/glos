//! `glos mcp`: the MCP server an agent host (Claude Code) starts over stdio, one per agent
//! session. It serves the review of the folder it was started in. A submitted review reaches the
//! agent as the answer to `wait_for_review`: the server watches the store for the submit.

use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::Result;
use rmcp::{
    ErrorData, RoleServer, ServerHandler, ServiceExt,
    handler::server::wrapper::Parameters,
    model::{ServerCapabilities, ServerConfig},
    schemars,
    service::{NotificationContext, RequestContext},
    tool, tool_handler, tool_router,
};
use rusqlite::{Connection, params};
use serde::Deserialize;

use crate::review::{self, Review, Round, Thread, Verdict, YOU};

const INSTRUCTIONS: &str = "glos is the user's code reviewer. When your work is ready, call \
request_review, then wait_for_review: it answers with the user's review when they submit it \
(call it again while it says it's still waiting). Address every comment, then request review \
again. get_review shows the review's state and threads at any time.";

/// How long one `wait_for_review` waits by default, staying under hosts' tool timeouts.
const WAIT: Duration = Duration::from_secs(240);

/// The review side of the server, without the protocol: what each tool does, given the store,
/// the folder served and the agent's name.
pub struct Agent {
    conn: Connection,
    folder: PathBuf,
    /// When this agent last looked, for "new" markers.
    seen: Option<String>,
}

impl Agent {
    pub fn new(conn: Connection, folder: PathBuf) -> Agent {
        Agent {
            conn,
            folder,
            seen: None,
        }
    }

    fn review(&self) -> Result<Option<Review>> {
        Review::for_folder(&self.conn, &self.folder)
    }

    /// Records this server in the store, and forgets servers whose process is gone.
    pub fn register(&self, name: &str) -> Result<()> {
        let pids: Vec<i64> = self
            .conn
            .prepare("SELECT pid FROM sessions")?
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        for pid in pids {
            if !alive(pid) {
                self.conn
                    .execute("DELETE FROM sessions WHERE pid = ?1", [pid])?;
            }
        }
        let now = review::now();
        self.conn.execute(
            "INSERT INTO sessions (pid, agent_name, folder, attached_at, last_seen)
             VALUES (?1, ?2, ?3, ?4, ?4)
             ON CONFLICT(pid) DO UPDATE SET agent_name = ?2, folder = ?3, last_seen = ?4",
            params![std::process::id(), name, self.folder.to_string_lossy(), now],
        )?;
        Ok(())
    }

    pub fn unregister(&self) {
        let _ = self
            .conn
            .execute("DELETE FROM sessions WHERE pid = ?1", [std::process::id()]);
    }

    fn touch(&mut self) {
        let now = review::now();
        let _ = self.conn.execute(
            "UPDATE sessions SET last_seen = ?2 WHERE pid = ?1",
            params![std::process::id(), now],
        );
        self.seen = Some(now);
    }

    /// `get_review`: a summary, or one thread in full. Only what the agent may see: comments
    /// you've submitted, and the agents' own.
    pub fn get_review(&mut self, thread: Option<&str>) -> Result<String> {
        let Some(r) = self.review()? else {
            return Ok(format!(
                "There's no review for {} yet. Call request_review when your work is ready.",
                self.folder.display()
            ));
        };
        let threads = visible(review::threads(&self.conn, &r.id)?);
        let seen = self.seen.clone();
        let is_new = |t: &Thread| {
            t.comments
                .iter()
                .any(|c| c.author == YOU && seen.as_ref().is_none_or(|s| &c.created_at > s))
        };
        let out = match thread {
            Some(id) => match threads.iter().find(|t| t.id == id) {
                Some(t) => full_thread(t),
                None => format!("No thread {id} in this review."),
            },
            None => {
                let mut out = format!("Review {} of {} ({}).\n", r.id, r.describe(), r.status);
                match review::rounds(&self.conn, &r.id)?.last() {
                    Some(round) => {
                        out += &format!(
                            "Last submitted round: {}, {}, with {} comment{}.\n",
                            round.n,
                            round.verdict.describe(),
                            round.comments,
                            plural(round.comments)
                        )
                    }
                    None => out += "Nothing submitted yet.\n",
                }
                if let Some(at) = &r.requested_at {
                    out += &format!(
                        "Review requested by {} at {at}; the user hasn't submitted since.\n",
                        r.requested_by.as_deref().unwrap_or("an agent")
                    );
                }
                match r.source() {
                    Ok(s) => {
                        out += &format!("Files changed ({}):\n", s.changes.len());
                        for c in &s.changes {
                            out += &format!("- {} ({:?})\n", c.path, c.kind());
                        }
                    }
                    Err(e) => out += &format!("Files: can't read them ({e:#}).\n"),
                }
                let open: Vec<&Thread> = threads.iter().filter(|t| t.status == "open").collect();
                out += &format!("Open threads ({}):\n", open.len());
                for t in open {
                    let first = &t.comments[0];
                    out += &format!(
                        "- {} {} {}: {} ({} comment{}{})\n",
                        t.id,
                        place(t),
                        first.author,
                        first.body.lines().next().unwrap_or(""),
                        t.comments.len(),
                        plural(t.comments.len()),
                        if is_new(t) { ", new" } else { "" }
                    );
                }
                out += "Call get_review with a thread_id for a whole thread.";
                out
            }
        };
        self.touch();
        Ok(out)
    }

    /// `request_review`: marks the work ready. In a git repo with no review yet, the review is
    /// made here, so `glos` opens on it.
    pub fn request_review(&mut self, name: &str, note: &str, files: &[String]) -> Result<String> {
        let r = match self.review()? {
            Some(r) => r,
            None => match crate::source::Source::git(&self.folder, None) {
                Ok(s) => Review::open(&self.conn, &s)?,
                Err(_) => {
                    return Ok(format!(
                        "{} isn't a git repository and has no review. Ask the user to run \
                         `glos diff <old> <new>` on what to review, then call request_review again.",
                        self.folder.display()
                    ));
                }
            },
        };
        r.request(&self.conn, name, note, files)?;
        self.touch();
        Ok(format!(
            "Review requested for {}. If glos is open there, it shows the request now; if not, ask \
             the user to run `glos` in {}. Then call wait_for_review to receive the review.",
            r.describe(),
            r.folder
        ))
    }

    /// One look for a submitted round nobody has received yet: the review as the answer.
    pub fn try_take(&mut self, name: &str) -> Result<Option<String>> {
        let Some(r) = self.review()? else {
            return Ok(None);
        };
        let taken = review::take_delivery(&self.conn, &r.id, name)?;
        self.touch();
        Ok(taken.map(|(round, threads)| delivery(&r, &round, &threads)))
    }
}

/// Comments the agent may see: submitted ones, and every agent's own.
fn visible(threads: Vec<Thread>) -> Vec<Thread> {
    threads
        .into_iter()
        .filter_map(|mut t| {
            t.comments
                .retain(|c| c.author != YOU || c.submitted_in.is_some());
            (!t.comments.is_empty()).then_some(t)
        })
        .collect()
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

fn place(t: &Thread) -> String {
    t.anchor
        .as_ref()
        .map_or("(on the whole review)".into(), |a| format!("on {a}"))
}

/// The anchored lines, quoted, at most three.
fn quote(t: &Thread) -> String {
    let Some(text) = &t.anchor_text else {
        return String::new();
    };
    let mut out: String = text.lines().take(3).map(|l| format!("  > {l}\n")).collect();
    if text.lines().count() > 3 {
        out += "  > …\n";
    }
    out
}

fn full_thread(t: &Thread) -> String {
    let mut out = format!("Thread {} {} ({}):\n{}", t.id, place(t), t.status, quote(t));
    for c in &t.comments {
        let edited = if c.edited_at.is_some() {
            " (edited)"
        } else {
            ""
        };
        out += &format!("{}{edited}:\n{}\n", c.author, indent(&c.body));
    }
    out
}

fn indent(body: &str) -> String {
    body.lines()
        .map(|l| format!("  {l}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The answer `wait_for_review` gives when the user submits.
fn delivery(r: &Review, round: &Round, threads: &[Thread]) -> String {
    let mut out = format!(
        "The user submitted their review of {} (round {}): {}.\n",
        r.describe(),
        round.n,
        round.verdict.describe()
    );
    let n: usize = threads.iter().map(|t| t.comments.len()).sum();
    if n > 0 {
        out += &format!("\n{n} comment{}:\n", plural(n));
    }
    for t in threads {
        out += &format!("\n[thread {}] {}\n{}", t.id, place(t), quote(t));
        for c in &t.comments {
            let edited = if c.edited_at.is_some() {
                " (edited since you last saw it)"
            } else {
                ""
            };
            out += &format!("{}{edited}:\n{}\n", c.author, indent(&c.body));
        }
    }
    out += "\n";
    out += match round.verdict {
        Verdict::Approved => "The work is approved: finish up.",
        Verdict::Changes => {
            "Changes are requested: address every comment, then call request_review and \
             wait_for_review again."
        }
        Verdict::Comment => {
            "These are comments without a verdict: answer them or take them into account, then \
             call request_review when you want another look."
        }
    };
    out
}

fn alive(pid: i64) -> bool {
    // SAFETY: signal 0 only checks that the process exists.
    let r = unsafe { libc::kill(pid as libc::pid_t, 0) };
    r == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[derive(Deserialize, schemars::JsonSchema)]
struct GetReview {
    /// A thread's id, for the whole thread instead of the summary.
    thread_id: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct RequestReview {
    /// What's ready and anything the reviewer should know, in a sentence or two.
    note: Option<String>,
    /// The files this work touched, if the review should focus on them.
    files: Option<Vec<String>>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct WaitForReview {
    /// How long to wait before answering "still waiting", in seconds (default 240, at most 600).
    timeout_secs: Option<u64>,
}

/// The protocol side; rmcp builds the tool router from `#[tool_router]` on each call.
#[derive(Clone)]
struct Server {
    agent: Arc<Mutex<Agent>>,
}

/// The agent's name, from the `clientInfo` its MCP client sent.
fn client_name(ctx: &RequestContext<RoleServer>) -> String {
    ctx.peer
        .peer_info()
        .map_or("agent".into(), |i| i.client_info.name.clone())
}

fn internal(e: anyhow::Error) -> ErrorData {
    tracing::error!("{e:#}");
    ErrorData::internal_error(format!("{e:#}"), None)
}

#[tool_router]
impl Server {
    #[tool(
        description = "The user's review of your work in this folder: the last verdict, the files, and the open threads (comments the user has submitted, with what's new since your last call). Pass thread_id for one whole thread."
    )]
    async fn get_review(&self, Parameters(p): Parameters<GetReview>) -> Result<String, ErrorData> {
        let mut a = self.agent.lock().unwrap();
        a.get_review(p.thread_id.as_deref()).map_err(internal)
    }

    #[tool(
        description = "Say your work is ready for the user to review. Afterwards call wait_for_review to receive the review."
    )]
    async fn request_review(
        &self,
        Parameters(p): Parameters<RequestReview>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, ErrorData> {
        let mut a = self.agent.lock().unwrap();
        a.request_review(
            &client_name(&ctx),
            p.note.as_deref().unwrap_or(""),
            &p.files.unwrap_or_default(),
        )
        .map_err(internal)
    }

    #[tool(
        description = "Wait for the user to submit their review, and answer with it: the verdict and every comment with its file, lines and thread id. If they haven't submitted within the timeout it answers that it's still waiting: call it again."
    )]
    async fn wait_for_review(
        &self,
        Parameters(p): Parameters<WaitForReview>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, ErrorData> {
        let name = client_name(&ctx);
        let wait = p
            .timeout_secs
            .map_or(WAIT, |s| Duration::from_secs(s.clamp(1, 600)));
        let until = Instant::now() + wait;
        loop {
            // The store is checked about twice a second; the lock isn't held while sleeping.
            if let Some(answer) = self
                .agent
                .lock()
                .unwrap()
                .try_take(&name)
                .map_err(internal)?
            {
                return Ok(answer);
            }
            if Instant::now() >= until || ctx.ct.is_cancelled() {
                return Ok(
                    "The user hasn't submitted the review yet. Call wait_for_review again \
                           to keep waiting."
                        .into(),
                );
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }
}

#[tool_handler]
impl ServerHandler for Server {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions(INSTRUCTIONS)
    }

    async fn on_initialized(&self, ctx: NotificationContext<RoleServer>) {
        let name = ctx
            .peer
            .peer_info()
            .map_or("agent".into(), |i| i.client_info.name.clone());
        if let Err(e) = self.agent.lock().unwrap().register(&name) {
            tracing::error!("registering the session: {e:#}");
        }
    }
}

/// Runs the server on stdin and stdout until the host closes it.
pub fn run(conn: Connection) -> Result<()> {
    let folder = std::env::current_dir()?.canonicalize()?;
    let agent = Arc::new(Mutex::new(Agent::new(conn, folder)));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let result = runtime.block_on(async {
        let server = Server {
            agent: agent.clone(),
        }
        .serve(rmcp::transport::stdio())
        .await?;
        server.waiting().await?;
        anyhow::Ok(())
    });
    agent.lock().unwrap().unregister();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::review::Anchor;

    fn setup() -> (tempfile::TempDir, Agent, Review) {
        let dir = tempfile::tempdir().unwrap();
        let (o, n) = (dir.path().join("o"), dir.path().join("n"));
        std::fs::create_dir_all(&o).unwrap();
        std::fs::create_dir_all(&n).unwrap();
        std::fs::write(o.join("a.rs"), "one\n").unwrap();
        std::fs::write(n.join("a.rs"), "one\ntwo\nthree\nfour\n").unwrap();
        let db = dir.path().join("glos.db");
        let conn = crate::store::open(&db).unwrap();
        let r = Review::open(&conn, &crate::source::Source::diff(&o, &n).unwrap()).unwrap();
        let agent = Agent::new(crate::store::open(&db).unwrap(), n.canonicalize().unwrap());
        (dir, agent, r)
    }

    fn at(start: u32, end: u32) -> Anchor {
        Anchor {
            path: "a.rs".into(),
            start,
            end,
        }
    }

    const FILE: &str = "one\ntwo\nthree\nfour\n";

    #[test]
    fn unsent_comments_stay_private() {
        let (_d, mut a, r) = setup();
        review::comment(&a.conn, &r.id, Some((&at(2, 2), FILE)), YOU, "draft-ish").unwrap();
        let out = a.get_review(None).unwrap();
        assert!(out.contains("Open threads (0)"), "{out}");
        assert!(!out.contains("draft-ish"));
        assert!(out.contains("Nothing submitted yet"));
        assert!(out.contains("- a.rs (Modified)"), "{out}");
        review::submit(&a.conn, &r.id, Verdict::Changes, &[], None).unwrap();
        let out = a.get_review(None).unwrap();
        assert!(out.contains("Open threads (1)"), "{out}");
        assert!(
            out.contains("on a.rs:2 you: draft-ish (1 comment)"),
            "{out}"
        );
        assert!(out.contains("Last submitted round: 1, changes requested, with 1 comment"));
        // Not new any more once seen.
        let t = &review::threads(&a.conn, &r.id).unwrap()[0];
        let full = a.get_review(Some(&t.id)).unwrap();
        assert!(full.contains("  > two\nyou:\n  draft-ish"), "{full}");
        assert!(
            a.get_review(Some("nope"))
                .unwrap()
                .contains("No thread nope")
        );
    }

    #[test]
    fn new_threads_are_marked_until_seen() {
        let (_d, mut a, r) = setup();
        a.get_review(None).unwrap();
        // Timestamps must move on past the agent's look.
        std::thread::sleep(Duration::from_millis(5));
        review::comment(&a.conn, &r.id, None, YOU, "overall").unwrap();
        review::submit(&a.conn, &r.id, Verdict::Comment, &[], None).unwrap();
        let out = a.get_review(None).unwrap();
        assert!(
            out.contains("(on the whole review) you: overall (1 comment, new)"),
            "{out}"
        );
        let out = a.get_review(None).unwrap();
        assert!(!out.contains(", new"), "{out}");
    }

    #[test]
    fn a_request_then_a_submit_reaches_the_waiting_agent_once() {
        let (_d, mut a, r) = setup();
        let out = a
            .request_review("claude", "parser done", &["a.rs".into()])
            .unwrap();
        assert!(out.contains("Then call wait_for_review"), "{out}");
        let rr = r.reload(&a.conn).unwrap();
        assert_eq!(rr.request_note.as_deref(), Some("parser done"));
        assert!(
            a.try_take("claude").unwrap().is_none(),
            "nothing submitted yet"
        );

        review::comment(
            &a.conn,
            &r.id,
            Some((&at(2, 4), FILE)),
            YOU,
            "split\nthis up",
        )
        .unwrap();
        review::comment(&a.conn, &r.id, None, YOU, "nice overall").unwrap();
        review::submit(&a.conn, &r.id, Verdict::Changes, &[], None).unwrap();
        let answer = a.try_take("claude").unwrap().unwrap();
        assert!(answer.contains("(round 1): changes requested"), "{answer}");
        assert!(answer.contains("2 comments"), "{answer}");
        assert!(
            answer
                .contains("] on a.rs:2-4\n  > two\n  > three\n  > four\nyou:\n  split\n  this up"),
            "{answer}"
        );
        assert!(
            answer.contains("(on the whole review)\nyou:\n  nice overall"),
            "{answer}"
        );
        assert!(answer.contains("address every comment"));
        assert!(a.try_take("claude").unwrap().is_none(), "delivered once");
    }

    #[test]
    fn an_edited_comment_is_sent_again_marked() {
        let (_d, mut a, r) = setup();
        review::comment(&a.conn, &r.id, None, YOU, "v1").unwrap();
        review::submit(&a.conn, &r.id, Verdict::Comment, &[], None).unwrap();
        a.try_take("claude").unwrap().unwrap();
        let c = review::threads(&a.conn, &r.id).unwrap()[0].comments[0]
            .id
            .clone();
        review::edit(&a.conn, &c, "v2").unwrap();
        review::submit(&a.conn, &r.id, Verdict::Approved, &[], None).unwrap();
        let answer = a.try_take("claude").unwrap().unwrap();
        assert!(
            answer.contains("you (edited since you last saw it):\n  v2"),
            "{answer}"
        );
        assert!(answer.ends_with("The work is approved: finish up."));
    }

    #[test]
    fn no_review_yet() {
        let dir = tempfile::tempdir().unwrap();
        let conn = crate::store::open(&dir.path().join("glos.db")).unwrap();
        let mut a = Agent::new(conn, dir.path().canonicalize().unwrap());
        assert!(a.get_review(None).unwrap().contains("no review for"));
        assert!(a.try_take("claude").unwrap().is_none());
        // Not git: the agent is told how the user starts a review.
        let out = a.request_review("claude", "", &[]).unwrap();
        assert!(out.contains("glos diff"), "{out}");
    }

    #[test]
    fn sessions_register_and_dead_ones_are_dropped() {
        let (_d, a, _r) = setup();
        a.conn
            .execute(
                "INSERT INTO sessions (pid, agent_name, folder, attached_at, last_seen)
                 VALUES (999999, 'gone', '/x', 'then', 'then')",
                [],
            )
            .unwrap();
        a.register("claude-code").unwrap();
        let names: Vec<String> = a
            .conn
            .prepare("SELECT agent_name FROM sessions")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(names, ["claude-code"]);
        a.unregister();
        let n: i64 = a
            .conn
            .query_row("SELECT count(*) FROM sessions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
    }
}
