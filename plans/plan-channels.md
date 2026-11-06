# Channels: push the review to Claude Code

**Status:** Blocked (2026-11-06) until Claude Code releases channels as a stable feature. Today
they're a research preview. They need a launch flag and an organisation setting, and Claude Code
drops channel messages silently when either is missing. Check the Claude Code release notes for
channels leaving the preview, then move this plan to `Ready to build`.

Today an agent asks for a review with `request_review` and then calls `wait_for_review`, which
blocks until you submit, or answers that you haven't submitted yet so that the agent calls it
again. While it waits, the session is busy doing nothing.

A Claude Code channel lets an MCP server push a message into a session. With a channel, the agent
can request a review and stop. When you submit, glos pushes the review into the session as a new
message, and the agent starts working on it.

Channels add to `wait_for_review`. They don't replace it. Agents other than Claude Code, sessions
without channels, and the CLI keep working as they do today.

## What M0 found

The M0 spike in `spikes/channel/` (deleted in 74f2eee, so look at the commit before it) showed
the server side works:

- The server announces the `claude/channel` experimental capability and sends
  `notifications/claude/channel`.
- The MCP protocol version was capped at 2025-11-25 for this to work.
- The push reached the Claude Code handshake, but delivery was never proven, because this
  account's organisation policy blocks channels.

`spikes/channel/FINDINGS.md` in that commit has the details.

## How it works

1. **Find out if the session takes channel messages.** When an MCP client connects, `glos mcp`
   (`src/mcp.rs`) records whether it said it supports channels. The client's capabilities are in
   the `initialize` request.
2. **Push on submit.** When you submit (`space s`, `:submit` or `glos submit`), the review's
   answer goes to every connected session of the review's agent that supports channels. The
   answer is the same text `wait_for_review` returns today. It's built in one place, so the two
   can't drift. The answer is marked as delivered only once it's sent, so a session that also
   calls `wait_for_review` doesn't get it twice.
3. **Tell the agent it can stop.** `request_review`'s answer says whether the review will be
   pushed. If it will, the agent can end its turn. If not, it calls `wait_for_review` as it does
   today.
4. **Delivery is uncertain.** Claude Code can drop a channel message without telling the server.
   glos keeps the answer until a session acknowledges it by calling `get_review` or
   `wait_for_review`, and `glos doctor` reports whether channels look usable.

The UI process and `glos mcp` share only the database (see "Processes and how they talk" in
`completed/plan-overview.md`). The MCP server already checks the store about twice a second
while it waits. The push uses the same check, run in the background while the server is
connected, rather than a new channel between the processes.

## Steps

1. Repeat the M0 spike against the released Claude Code, and prove a message is delivered.
2. Record channel support per session, and push on submit.
3. Change `request_review`'s answer, the MCP server's instructions, and the README's "Connecting
   your agent" section.
4. Add the check to `glos doctor`.

## Tests

- With a test client that supports channels, a submit sends exactly one push, and a later
  `wait_for_review` doesn't return the same review again.
- With a client that doesn't, nothing is pushed and `wait_for_review` works as today.
- `request_review`'s answer says whether the review will be pushed, for both kinds of client.

## Open

- Should glos push other events too, such as a reply you write after submitting? Today the agent
  only hears from you when you submit.

## Relevant code

- `src/mcp.rs`: the server's capabilities, `request_review`, `wait_for_review` and the agent's
  sessions.
- `src/doctor.rs`.
- `README.md`, "Connecting your agent".
