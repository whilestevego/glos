//! M0 spike 1: does a Claude Code channel push from an rmcp stdio server reach the session?
//!
//! Two triggers:
//! - the `push_later` tool: push `text` after `delay_secs`, after the tool call has returned;
//! - an inbox file (`$SPIKE_INBOX`, default `<crate>/inbox.txt`): when it appears, its content is
//!   pushed and the file deleted. This stands in for glos's UI writing a submit to the database.
//!
//! Everything is logged to `<crate>/spike.log`; stdout carries the protocol.

use std::{borrow::Cow, io::Write, path::PathBuf, time::Duration};

use rmcp::{
    ErrorData, RoleServer, ServerHandler, ServiceExt,
    handler::server::{tool::ToolRouter, wrapper::Parameters},
    model::{
        CustomNotification, ProtocolVersion, ServerCapabilities, ServerConfig, ServerNotification,
    },
    schemars,
    service::{NotificationContext, Peer, RequestContext},
    tool, tool_handler, tool_router,
};
use serde::Deserialize;

fn log(msg: impl AsRef<str>) {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/spike.log");
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "[{}] {}", std::process::id(), msg.as_ref());
    }
}

async fn push(peer: &Peer<RoleServer>, content: &str, kind: &str) {
    let params = serde_json::json!({
        "content": content,
        "meta": { "kind": kind, "review_id": "spike-1" },
    });
    let n = CustomNotification::new("notifications/claude/channel", Some(params));
    match peer.send_notification(ServerNotification::CustomNotification(n)).await {
        Ok(()) => log(format!("pushed ({kind}): {content}")),
        Err(e) => log(format!("push failed ({kind}): {e}")),
    }
}

#[derive(Deserialize, schemars::JsonSchema)]
struct PushLater {
    text: String,
    delay_secs: u64,
}

#[derive(Clone)]
struct Spike {
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl Spike {
    #[tool(description = "Push `text` back into this session as a channel message after `delay_secs`.")]
    async fn push_later(
        &self,
        Parameters(PushLater { text, delay_secs }): Parameters<PushLater>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, ErrorData> {
        let peer = ctx.peer.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(delay_secs)).await;
            push(&peer, &text, "delayed").await;
        });
        Ok(format!("scheduled in {delay_secs}s"))
    }
}

#[tool_handler]
impl ServerHandler for Spike {
    fn get_info(&self) -> ServerConfig {
        let mut caps = ServerCapabilities::builder().enable_tools().build();
        caps.experimental = Some([("claude/channel".to_string(), Default::default())].into());
        ServerConfig::new(caps).with_instructions(
            "glos spike. Messages arrive as <channel source=\"glos\" kind=... review_id=...>. \
             When one arrives, say in one line what it contained. Don't call tools in response.",
        )
    }

    // Revision 2026-07-28 drops `initialize` and can't carry custom notifications, so channel
    // pushes would be refused. Cap negotiation at the last revision with a handshake.
    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        Cow::Borrowed(ProtocolVersion::known_up_to(&ProtocolVersion::LATEST_WITH_INITIALIZE))
    }

    async fn on_initialized(&self, ctx: NotificationContext<RoleServer>) {
        let info = ctx.peer.peer_info();
        log(format!(
            "initialized: client={:?} protocol={:?}",
            info.as_ref().map(|i| &i.client_info),
            info.as_ref().map(|i| &i.protocol_version),
        ));
        let peer = ctx.peer.clone();
        let inbox: PathBuf = std::env::var_os("SPIKE_INBOX")
            .map(Into::into)
            .unwrap_or_else(|| concat!(env!("CARGO_MANIFEST_DIR"), "/inbox.txt").into());
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(500)).await;
                if let Ok(text) = std::fs::read_to_string(&inbox) {
                    let _ = std::fs::remove_file(&inbox);
                    push(&peer, text.trim(), "inbox").await;
                }
            }
        });
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    log("starting");
    let server = Spike { tool_router: Spike::tool_router() }
        .serve(rmcp::transport::stdio())
        .await?;
    server.waiting().await?;
    log("exiting");
    Ok(())
}
