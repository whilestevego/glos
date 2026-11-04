# glos

A terminal code reviewer for work done by coding agents. It feels like Helix, runs locally, and
talks to agents both ways: you comment, the agent answers on the same line, and the review stays
open across sessions until every thread is settled.

Why it exists: reviewing an agent's branch on GitHub means pushing, opening a PR and switching to a
browser for code you didn't write and don't want to publish yet. `tuicr` gets closer but has no way
for agents to plug in, needs a manual save before quitting, and doesn't feel like Helix. Every
decision behind this plan, with the options considered, is in [DECISIONS.md](DECISIONS.md).

## Goals

- Review an agent's work without leaving the terminal or pushing anything.
- Feel like Helix: same modes, keys, pickers, themes and syntax highlighting.
- Agents are first-class: they reply, resolve, pre-annotate their changes, ask for review and can
  point you at a line, through MCP.
- Nothing you write is ever lost: every comment is saved the moment you write it.
- Lean on well-maintained crates; write only what's specific to reviewing.

## Non-goals

- Replacing Helix. Edits in glos are for quick fixes; bigger work stays in your editor.
- Posting to GitHub, GitLab or any other forge.
- Windows support.

## The experience

1. An agent finishes a task and calls `request_review`. Through the Ghostty integration, glos opens
   in a split showing the branch since it left `main`, uncommitted and untracked files included.
   You can also run `glos` yourself in any repo, or open it from Claude Code with a slash command.
2. The agent may already have left notes on its own changes, such as "why I did this" or "unsure
   here". They're drawn in their own colour.
3. You read the code in full-file view, with changed lines marked in the gutter. `]h` jumps to the
   next hunk and `]f` to the next file. `space f` opens a fuzzy picker of changed files.
4. Select lines the Helix way (`x`, `X`, `v`, or a mouse drag), then `space c` to comment. Each
   comment saves instantly.
5. `space s` submits. The review is pushed straight into the agent's Claude Code session through an
   MCP channel. There's no prompt to paste.
6. As the agent works, glos refreshes live: the diff changes, threads follow their lines, and
   replies appear in place with a notice. A thread whose code is gone is marked outdated.
7. Round 2: switch the scope to "since last review" to see only what changed in response to you.
   Reply, resolve, or reopen, then submit again.
8. With several agents on several branches or worktrees, the dashboard lists every review with
   pending work and open threads, and you jump between them.

## Architecture

One Rust binary, organised as a Cargo workspace of focused crates:

| Crate | Job |
|---|---|
| `glos` (bin) | CLI entry point: `glos`, `glos mcp`, `glos review …`, `glos dashboard` |
| `glos-core` | Review data model: reviews, rounds, threads, comments, anchors, and re-anchoring |
| `glos-git` | Diffs from gix: branch vs base, since last review, per commit, untracked files |
| `glos-store` | Persistence in the user data dir; atomic writes; file lock |
| `glos-ui` | ratatui front end: views, keymap engine, pickers, popups, themes, highlighting |
| `glos-edit` | The in-place editing buffer (ropey) and the Helix selection model |
| `glos-rpc` | Unix-socket protocol between running UIs, the MCP server and the CLI |
| `glos-mcp` | MCP server for agents, including the channel push |

### Processes and how they talk

- **The UI** (`glos`) is the only process that draws. It opens a Unix socket at
  `$XDG_RUNTIME_DIR/glos/<review-id>.sock`, or the data dir on macOS.
- **The MCP server** (`glos mcp`) is started by the agent host (Claude Code) over stdio, one per
  agent session. It reads and writes review state through the store, and through the socket when a
  UI is open, so replies appear live. It's also the channel that pushes your submitted review into
  that agent's session.
- **The CLI** (`glos review …`) reads and writes the same state as JSON for scripts and for agents
  without MCP.
- The store is the source of truth. The socket is for live notifications only, so nothing breaks
  if a UI isn't open.

### Data model (`glos-core`)

```
Review     id, repo path, worktree path, branch, base, created/updated, rounds[], threads[]
Round      n, submitted_at, base_commit, head_commit, snapshot of the worktree (tree id)
Thread     id, anchor, status (open | resolved | outdated), comments[], created_round
Anchor     file path, side (new | old), line range, the anchored lines' text, context lines
           (3 above, 3 below), commit the anchor was taken at; or none for a review-level thread
Comment    id, author (you | agent name), kind (comment | annotation), body, created_at
```

**Re-anchoring.** When the code changes, each thread is placed again: the old position is mapped
through the diff between the snapshot it was anchored in and now (with imara-diff). If the
anchored text is gone, glos searches the file for the same text near the old spot, using the
context lines as a tie-break. If that fails, the thread becomes **outdated** and is shown against
the code as it was. This is the riskiest logic in the project and gets the most tests.

### Storage (`glos-store`)

- Location: the user data dir from `etcetera` (`~/.local/share/glos` on Linux; macOS uses its
  Application Support folder), one folder per repo (keyed by the repo's absolute path hash), one
  JSON file per review (one per branch or worktree).
- Every change is written immediately with an atomic rename, so there's no "save" step and quitting
  never loses anything.
- An advisory file lock (`fs4`) guards writes, because the UI, the MCP server and the CLI share it.

## Viewing the code

Three views, all on the same review. `space v` (or `:view <name>`) switches between them and keeps
your place:

- **Full file + gutter (default):** the whole new file, changed lines marked in the gutter as Helix
  does. Removed lines can be shown inline under a change on demand.
- **Unified:** classic hunks with context, best in narrow panes.
- **Side by side:** old and new in two columns.

Scopes, switched with `space o` or `:scope`, which you can combine with a files filter:

- **Branch vs base** (default): the merge-base with `main` (or origin's default branch) against the
  worktree, uncommitted and untracked files included.
- **Since last review:** against the snapshot taken at your last submit.
- **Per commit:** step through the branch's commits with `]c` / `[c`.
- **Files filter:** by path or glob, or only files with open threads.

Navigation: the file picker (`space f`) is primary; `space e` toggles a file tree with change counts
and thread badges. Threads are drawn inline under their lines as virtual text, collapsed to a
one-line marker until you open them, like Helix's inline diagnostics.

## Keymap (Helix first)

glos follows Helix's keys wherever an action exists there: `hjkl`, `w`/`b`/`e`, `gg`/`ge`, `gh`/`gl`,
`x`/`X`, `v` (select mode), `%`, `;`, `,`, `/` search, `u` undo, `:` commands, `space`-mode, and so on.
Review actions are added where Helix leaves room:

| Key | Action |
|---|---|
| `space c` | Comment on the selection (on the line when nothing is selected) |
| `space C` | Review-level comment |
| `space r` | Reply to the thread under the cursor |
| `space x` | Resolve or reopen the thread under the cursor |
| `space s` | Submit the review to the agent |
| `space v` | Switch view |
| `space o` | Switch scope |
| `space f` | Changed-file picker |
| `space t` | Thread picker (open, outdated, by author) |
| `space e` | Toggle file tree |
| `space d` | Dashboard |
| `]h` `[h` | Next / previous hunk |
| `]t` `[t` | Next / previous thread |
| `]f` `[f` | Next / previous file |
| `]c` `[c` | Next / previous commit (per-commit scope) |
| `enter` / `za` | Expand or collapse the thread under the cursor |
| `i` `a` `o` … | Edit in place on the new side (Helix insert mode); changes save on `:w` |

Every action is also a typable command (`:comment`, `:reply`, `:resolve`, `:submit`, `:view`,
`:scope`, `:dashboard`), with completion. Pressing `space`, `g`, `]` or `[` shows a which-key popup.
Keys can be remapped in `config.toml`, as in Helix.

**Mouse (full):** click to move the cursor, drag to select, wheel to scroll; click files in the tree
and picker, click a thread marker to expand it, and click buttons in popups. The keyboard stays
primary.

## Helix fidelity

- **Themes and config:** read Helix theme TOML files from the Helix runtime and from
  `~/.config/helix/themes`. glos's own `config.toml` sits in `~/.config/glos/` and mirrors Helix's
  layout (`theme`, `[keys.normal]`, `[editor]`). If no theme is set, use the one Helix's config sets.
- **Syntax highlighting:** tree-sitter, reusing Helix's runtime: compiled grammars in
  `runtime/grammars/*.so`, highlight queries in `runtime/queries/<lang>/`, and `languages.toml`
  (built into Helix, overridable in `~/.config/helix/languages.toml`) to map files to languages.
  Runtime directories are found the way `hx --health` lists them: `~/.config/helix/runtime`, then
  the install's own (here `/opt/homebrew/Cellar/helix/25.07.1/libexec/runtime`). Without Helix
  installed, glos falls back to plain text and says so.
- **Editing:** a ropey buffer with Helix's selection-first model (selections, multiple cursors,
  `insert`/`normal`/`select` modes). Writes go to the worktree file, which triggers a live refresh
  like any other change.

## Agents: the MCP server

`glos mcp` speaks MCP over stdio (crate `rmcp`). Tools:

| Tool | What it does |
|---|---|
| `get_review` | The current review: scope, files, open threads, with anchors and bodies |
| `reply` | Reply on a thread |
| `resolve` | Resolve a thread with a note; `reopen` reverses it |
| `annotate` | Leave an agent note on a line or range before you review |
| `request_review` | Say the work is ready; opens or notifies glos (via the Ghostty integration), optionally scoped to the files it touched |
| `focus` | Move your open view to a file and line. Only works when you've allowed it, per review, and shows who moved it |
| `wait_for_review` | Fallback for hosts without channels: blocks until you submit |

**Channel push.** When you submit, the MCP server attached to that worktree's agent session pushes
the review into the session as a channel message (Claude Code channels: the server declares the
channel capability and sends a notification). The message holds every new comment with its file,
lines and thread id, and tells the agent to answer each one with `reply`/`resolve`. This needs
checking first (see M0): if channels aren't available or behave differently, the fallback is a
small Claude Code mod that listens on the socket and submits the review as a prompt.

**Which session gets it:** each review is tied to a worktree. The MCP server registers the worktree
it was started in; a submit goes to the sessions registered for that worktree. With none attached,
the review waits and is delivered when one attaches.

## The CLI

For scripts and agents without MCP, every operation exists as a subcommand with JSON output:
`glos review list | show | comments | reply | resolve | annotate | submit`. This is also how the
Claude Code mod fallback and tests drive glos.

## The dashboard

`glos dashboard`, or `space d` inside a review, lists every review on this machine: repo, branch or
worktree, the agent's state (working, waiting for review, replied), and open, outdated and new-reply
counts. `enter` opens one. It's built from the store plus live socket status, and refreshes on its
own.

## Launching

- **Standalone:** `glos` in a repo finds the branch, its base and any existing review.
- **From Claude Code:** a slash command opens glos in a zoomed Ghostty split, which is what the
  ghostty mod's `/tuicr` does today (it would be adapted).
- **When the agent asks:** `request_review` opens glos the same way, or notifies you when a split
  can't be opened.

## Crates

Picked for being widely used and maintained. Versions are fixed when the workspace is created.

| Need | Crate |
|---|---|
| Terminal UI | `ratatui`, `crossterm` (mouse, bracketed paste, focus events) |
| Text buffer | `ropey` |
| Syntax highlighting | `tree-sitter`, plus Helix's highlighter crate `tree-house` if its API suits; `tree-sitter-highlight` otherwise; `libloading` to load grammars |
| Fuzzy matching | `nucleo` (Helix's own matcher) |
| Git | `gix` |
| Line diff | `imara-diff` (used by Helix and gix) |
| File watching | `notify` with `notify-debouncer-full` |
| MCP | `rmcp` (the official Rust SDK) |
| Async and sockets | `tokio` |
| CLI | `clap` |
| Serialisation | `serde`, `serde_json`, `toml` |
| Paths | `etcetera` (what Helix uses) |
| Locking | `fs4` |
| Ids and time | `uuid`, `jiff` |
| Errors and logs | `anyhow`, `thiserror`, `tracing`, `tracing-subscriber` |
| Tests | `insta` (snapshots of rendered screens and JSON), `tempfile` |

## Milestones

The MVP is M0 through M5: reading and commenting, threads in both directions, all three views and
the dashboard.

- **M0, spikes.** Prove the three risky pieces before building around them:
  1. A Claude Code channel push from an `rmcp` server reaches a session as a message.
  2. A Helix runtime grammar and query highlight a file through `tree-house` or
     `tree-sitter-highlight`.
  3. gix produces the branch-vs-base diff including uncommitted and untracked files.
- **M1, the reader.** Store and data model, branch-vs-base diff, full-file + gutter view, Helix
  keymap engine with motions and selections, file picker, themes and highlighting, mouse.
- **M2, commenting.** Comments on selections, review-level comments, inline collapsible threads,
  instant saving, `space s` submit through the channel, the CLI.
- **M3, two-way.** MCP tools (reply, resolve, annotate, request review, focus), live refresh
  through `notify` and the socket, re-anchoring and outdated threads.
- **M4, views and scopes.** Unified and side by side; since last review, per commit, files filter;
  file tree.
- **M5, dashboard and launch.** Multi-review dashboard; Ghostty and Claude Code launching; the
  `/tuicr` command reworked as `/glos`.
- **After the MVP:** in-place editing with the Helix model (`glos-edit`), remappable keys, and
  polish.

In-place editing is listed after the MVP because the reviewer is useful without it, and it's the
largest piece of Helix to imitate. Move it earlier if quick fixes turn out to be needed daily.

## Open questions

- **Channels:** how stable Claude Code channels are, whether they need a launch flag, and what they
  allow, decided by the M0 spike.
- **`tree-house`:** whether Helix's highlighter crate is usable outside Helix, or
  `tree-sitter-highlight` is needed instead (M0).
- **Re-anchoring rules:** how far to search for moved text before calling a thread outdated.
- **Old-side comments:** whether comments on removed lines are allowed in full-file view, or only in
  the unified and side-by-side views.
