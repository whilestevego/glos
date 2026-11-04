# Spike 1: channel push from an rmcp server

rmcp 3.8.0, stdio. Docs: https://code.claude.com/docs/en/channels-reference.md and
https://code.claude.com/docs/en/channels.md.

## The contract (from the docs)

- Declare `capabilities.experimental["claude/channel"] = {}` in the initialize result.
- Push `notifications/claude/channel` with `{ content: string, meta?: {ident: string} }`. Each meta
  key becomes an attribute on the `<channel source="<server name>" ...>` tag; keys must be
  letters/digits/underscores or they're silently dropped.
- No ack. Events that the session didn't load as a channel are dropped silently. Busy sessions
  batch events into the next turn. Events only arrive while the session is open.
- Normal tools work alongside (`capabilities.tools`). `instructions` is delivered to Claude as
  context: say what the tag means and which tool to answer with.
- Enabling (research preview, flags hidden from `--help`): `claude
  --dangerously-load-development-channels server:<name>` for a server in `.mcp.json`, or
  `--channels plugin:<name>@<marketplace>` for a published plugin. Interactive sessions only
  (ignored with `-p` and the Agent SDK). claude.ai or Console auth; not Bedrock/Vertex/Foundry.
  Team/Enterprise orgs need an Owner to enable channels.

## What the spike shows

- rmcp sends it with `peer.send_notification(ServerNotification::CustomNotification(...))`, and the
  experimental capability is a `BTreeMap` on `ServerCapabilities`. No SDK changes needed.
- **Protocol trap:** rmcp 3.8 defaults to revision 2026-07-28, which drops `initialize` and refuses
  custom notifications; the docs say channel servers on that revision can't deliver. The server
  caps `supported_protocol_versions()` at `LATEST_WITH_INITIALIZE` (2025-11-25). Claude Code's
  `MCP_PROTOCOL_NEGOTIATION=legacy` is the client-side alternative.
- Driven by hand over stdio: initialize negotiates 2025-11-25, the capability is advertised,
  `clientInfo` is readable in `on_initialized` (for the Session's agent name), and both triggers
  push correctly-shaped notifications: a delayed push after a tool call returned, and an inbox
  file picked up by a 500 ms poll (stand-in for a submit seen through `PRAGMA data_version`).

## End-to-end test (needs a person: interactive session)

```sh
cd spikes/channel && cargo build
claude --dangerously-load-development-channels server:glos-spike
# in the session: "call push_later with text 'hello' and delay_secs 5", then wait
# from another shell, while the session is idle:
echo "review submitted: changes requested on src/main.rs:42" > spikes/channel/inbox.txt
```

Pass: both messages appear in the session as `<channel source="glos-spike" kind=... >` and Claude
answers them without being prompted. `spike.log` shows the negotiated protocol and each push.
If nothing arrives, retry with `MCP_PROTOCOL_NEGOTIATION=legacy` set.

Result (2026-11-04, Claude Code 2.1.318, Claude Team account): **blocked by org policy.** At
startup Claude Code says "--dangerously-load-development-channels blocked by org policy
(server:glos-spike). Inbound messages will be silently dropped. Have an administrator set
channelsEnabled: true in managed settings to enable". What it did confirm against the real client:
it spawns the server, sends `clientInfo` (`claude-code`, title "Claude Code") and settles on
protocol 2025-11-25 without `MCP_PROTOCOL_NEGOTIATION=legacy`, so the version cap is enough.
Delivery is still unproven; it needs an org Owner to enable channels, or a personal Pro/Max
account. Until then, `wait_for_review` is the delivery path glos can rely on.

Automated with a pty driver (no tmux):
`claude --mcp-config .mcp.json --strict-mcp-config --dangerously-load-development-channels server:glos-spike`.
