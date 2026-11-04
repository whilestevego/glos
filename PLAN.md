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
- Work on any two files or folders. Git adds branches, commits and a base, but isn't required.
- Feel like Helix: same modes, keys, pickers, themes and syntax highlighting.
- Agents are first-class: they reply, resolve, pre-annotate their changes, ask for review and can
  point you at a line, through MCP.
- Nothing you write is ever lost: every comment is saved the moment you write it.
- Standalone: runs in any terminal, with no terminal-specific integration required.
- Lean on well-maintained crates; write only what's specific to reviewing.

## Non-goals

- Replacing Helix. Edits in glos are for quick fixes; bigger work stays in your editor.
- Posting to GitHub, GitLab or any other forge.
- Depending on a particular terminal or multiplexer.
- A Claude Code plugin (slash commands, mods). That's a separate project built on glos's CLI and
  MCP server.
- Windows support.

## The experience

1. An agent finishes a task and calls `request_review`. If glos is open, the request appears
   there. If not, glos runs your `open_command` (a tmux or kitty split, for example) when you've
   set one; otherwise the agent asks you to run `glos`. In a git repo, glos shows the branch since
   it left `main`, uncommitted and untracked files included. You can also run `glos` yourself in
   any repo, or `glos old new` on any two files or folders.
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
7. Round 2: switch the scope to "since last review" to see only what changed in response to you,
   with or without git. Reply, resolve, or reopen, then submit again.
8. With several agents on several branches or worktrees, the dashboard lists every review with
   pending work and open threads, and you jump between them.

## What glos can review

- **Two files:** `glos old.rs new.rs`. The most basic review; no git needed.
- **Two folders:** `glos old/ new/`. Files are matched by relative path; `.gitignore` rules are
  honoured if present.
- **A git repo:** `glos` inside it. Branch vs base by default, plus per-commit scope.

Each source becomes the same list of (path, old text, new text), and glos diffs it itself with
imara-diff. Git is only one way to get the old side; everything after that is shared.

## Architecture

One Rust binary crate, split into modules. Split modules into crates later only if compile times
or reuse call for it.

| Module | Job |
|---|---|
| `cli` | Entry point: `glos`, `glos <old> <new>`, `glos mcp`, `glos review …`, `glos dashboard`, `glos open` |
| `core` | Review data model: reviews, rounds, snapshots, threads, comments, anchors, and re-anchoring |
| `source` | Turns two files, two folders or a git repo into old/new pairs; git through gix |
| `store` | The SQLite database in the user data dir, including snapshots |
| `ui` | ratatui front end: views, keymap engine, key help panel, pickers, popups, themes, highlighting |
| `mcp` | MCP server for agents, including the channel push |
| `edit` | After the MVP: in-place editing on top of `helix-core` |

### Processes and how they talk

- **The UI** (`glos`) is the only process that draws.
- **The MCP server** (`glos mcp`) is started by the agent host (Claude Code) over stdio, one per
  agent session. It registers its session and the folder it was started in, and reads and writes
  review state in the store. It's also the channel that pushes your submitted review into that
  agent's session.
- **The CLI** (`glos review …`) reads and writes the same state as JSON for scripts and for agents
  without MCP.
- All of them share one SQLite database, and that's the only channel between them: no socket and
  no daemon. Each process notices the others' changes by checking `PRAGMA data_version` about twice
  a second. That's how replies appear live in the UI, how an MCP server learns you submitted, and
  how `focus` reaches an open view.

### Data model (`core`)

```
Review     id, source (two files | two folders | git repo + worktree + branch + base),
           created/updated, rounds[], threads[]
Round      n, submitted_at, snapshot
Snapshot   the content of every file in the review at that moment: path → content hash
           (git: also the head commit, for reference)
Thread     id, anchor, status (open | resolved | outdated), comments[], created_round
Anchor     file path, line range on the new side, the anchored lines' text, context lines
           (3 above, 3 below), hash of the file's content when the comment was made;
           or none for a review-level thread
Comment    id, author (you | agent name), kind (comment | annotation), body, created_at
Session    agent session id, folder it serves, process id, last seen
```

**Snapshots.** glos keeps its own copy of what you reviewed instead of relying on git: an agent's
work is often uncommitted, there may be no git at all, and git objects nobody references get
garbage-collected. A round's snapshot is taken when you submit, and an anchor keeps the content of
its file when the comment is made. Each distinct content is stored once (by hash), and only for
files in the review.

**Re-anchoring.** When the code changes, each thread is placed again: the old position is mapped
through the diff between the file's content when the thread was anchored and now (with imara-diff).
If the anchored text is gone, glos searches the file for the same text near the old spot, using the
context lines as a tie-break. If that fails, the thread becomes **outdated** and is shown against
the code as it was. This is the riskiest logic in the project and gets the most tests.

While the agent is editing, a thread isn't marked outdated straight away. glos waits until the
files have been quiet for a few seconds, or the round ends, so threads don't flicker between open
and outdated mid-edit. Once outdated, a thread stays outdated until you reopen or resolve it.

Comments go on the new side only. Comments on removed lines may come later, in the unified and
side-by-side views.

### Storage (`store`)

- One SQLite database (`rusqlite`, bundled), `glos.db`, in the user data dir from `etcetera`
  (`~/.local/share/glos` on Linux; macOS uses its Application Support folder).
- Every action (a comment, a reply, a resolve) is one transaction, written the moment it happens.
  There's no "save" step, and quitting never loses anything.
- SQLite handles several processes writing at once (WAL mode with a busy timeout), so the UI, MCP
  servers and CLI never overwrite each other's changes.
- Reviews are keyed by their source: for git, the shared git dir (so worktrees of one repo sit
  together) plus the worktree and branch; for files and folders, their canonical paths.

## Viewing the code

Three views, all on the same review. `space v` (or `:view <name>`) switches between them and keeps
your place:

- **Full file + gutter (default):** the whole new file, changed lines marked in the gutter as Helix
  does. Removed lines can be shown inline under a change on demand.
- **Unified:** classic hunks with context, best in narrow panes.
- **Side by side:** old and new in two columns.

Scopes, switched with `space o` or `:scope`, which you can combine with a files filter:

- **Old vs new** (default for two files or folders).
- **Branch vs base** (default for git): the merge-base with `main` (or origin's default branch)
  against the worktree, uncommitted and untracked files included.
- **Since last review** (any source): against the snapshot taken at your last submit.
- **Per commit** (git only): step through the branch's commits with `]c` / `[c`.
- **Files filter:** by path or glob, or only files with open threads.

Navigation: the file picker (`space f`) is primary; `space e` toggles a file tree with change counts
and thread badges. Threads are drawn inline under their lines as virtual text, collapsed to a
one-line marker until you open them, like Helix's inline diagnostics.

## Keymap (Helix first)

glos follows Helix's keys wherever an action exists there: `hjkl`, `w`/`b`/`e`, `gg`/`ge`, `gh`/`gl`,
`x`/`X`, `v` (select mode), `%`, `;`, `,`, `/` search, `:` commands, `space`-mode, and so on. `u`
(undo) and the other editing keys come with in-place editing, after the MVP. Review actions are
added where Helix leaves room:

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
| `enter` | Expand or collapse the thread under the cursor |
| `i` `a` `o` … | After the MVP: edit in place on the new side (Helix insert mode); changes save on `:w` |

Some of these keys mean something else in Helix: `space c` toggles comments, `space s` is the
symbol picker, `space r` renames and `space d` is the diagnostics picker. Those need an LSP or
editing, which glos doesn't have, so glos reuses the keys for review actions on purpose.

Every action is also a typable command (`:comment`, `:reply`, `:resolve`, `:submit`, `:view`,
`:scope`, `:dashboard`), with completion. Keys can be remapped in `config.toml`, as in Helix.

**The key help panel** is essential, not an extra. As in Helix's infobox, pressing any key that
starts a sequence (`space`, `g`, `]`, `[`, `m`, `z`, …) opens a panel listing the keys that can
follow, each with a one-line description. glos goes one step further and lists only what applies
where you are: thread actions when the cursor is on a thread, `]c`/`[c` only in per-commit scope,
submit only when there's something to submit. The panel is built from the live keymap, so remapped
keys show their new bindings. It's part of the keymap engine in M1.

**Mouse (full):** click to move the cursor, drag to select, wheel to scroll; click files in the tree
and picker, click a thread marker to expand it, and click buttons in popups. The keyboard stays
primary.

## Helix fidelity

Everything glos takes from Helix is compiled into the binary from a pinned Helix release. glos
never reads Helix's files or config at run time, and works the same whether Helix is installed or
not.

- **Themes and config:** Helix's themes are embedded. glos's own `config.toml` sits in
  `~/.config/glos/` and mirrors Helix's layout (`theme`, `[keys.normal]`, `[editor]`). Your own
  themes go in `~/.config/glos/themes/` in Helix's format, so a Helix theme can be copied over as
  is. Themes that build on another (`inherits`) and named colours (`[palette]`) are resolved as
  Helix does.
- **Syntax highlighting:** tree-sitter, with the grammars compiled into glos. A script takes from a
  pinned Helix release its `languages.toml` (which maps files to languages and pins each grammar's
  source and revision), the highlight queries and the themes. It fetches each grammar's source at
  the revision Helix pins and vendors it in the repo. `build.rs` compiles the grammars with `cc`,
  and the queries, themes and `languages.toml` are embedded with `include_str!`. Using Helix's
  pinned revisions keeps queries and grammars matching. Queries that build on another language's
  (`; inherits:`) are followed as Helix does. Moving to a new Helix release means rerunning the
  script.
- **Languages:** a starting set, listed in one file: Rust, TypeScript/TSX, JavaScript, Python, Go,
  C, C++, Java, Ruby, Bash, JSON, TOML, YAML, Markdown, HTML, CSS, SQL and Lua. Adding one is a
  line plus a rerun of the script. Files in other languages show as plain text.
- **Licences:** Helix's queries, themes and `languages.toml` are MPL-2.0 and stay under it, with
  their notice, in their own folder. Grammars keep their own licences (mostly MIT).
- **Movement and selections come from Helix itself.** `helix-core` (plain functions over a ropey
  buffer, with no UI) provides motions, selections, multiple cursors and text objects. It's a git
  dependency pinned to the same Helix release as the grammars; if it pulls in too much, its
  movement, selection and text-object files are copied in instead, under MPL-2.0. glos never
  rewrites this behaviour, so `w`, `x`, `%`, `mi(` and counts behave exactly as in Helix, and
  Helix's own unit tests come with them.
- **Editing (after the MVP):** the same `helix-core` model with `insert`/`normal`/`select` modes.
  Writes go to the file on the new side, which triggers a live refresh like any other change.
- **The keymap checklist.** A script generates `KEYMAP.md` from Helix's default keymap at the
  pinned release: every Helix command, marked *done*, *not applicable* (LSP, buffers, windows) or
  *replaced* (with the glos action that took its key). That makes coverage checkable, and moving
  to a new Helix release shows exactly what changed.
- **The look.** These match Helix: the statusline layout and mode names (`NOR`/`INS`/`SEL`), the
  cursor shape per mode, the key help panel (Helix's infobox), the picker layout with its preview,
  the gutter's diff signs, the `:` command line with completion, and threads drawn like Helix's
  inline diagnostics. `insta` screen snapshots lock each one in.
- **Tests in Helix's notation.** Key-sequence tests mark selections in the text as Helix's tests do
  (`#[h|]#ello`): start text and keys in, expected text and selections out. Cases are copied from
  Helix's test suite where they apply.

## Agents: the MCP server

`glos mcp` speaks MCP over stdio (crate `rmcp`). Tools:

| Tool | What it does |
|---|---|
| `get_review` | The current review: scope, files, open threads, with anchors and bodies |
| `reply` | Reply on a thread |
| `resolve` | Resolve a thread with a note; `reopen` reverses it |
| `annotate` | Leave an agent note on a line or range before you review |
| `request_review` | Say the work is ready, optionally scoped to the files it touched. An open glos shows the request; otherwise glos runs `open_command` if set, and the answer tells the agent to ask you to run `glos` if nothing opened |
| `focus` | Move your open view to a file and line. Only works when you've allowed it, per review, and shows who moved it |
| `wait_for_review` | Fallback for hosts without channels: waits a few minutes for you to submit, then answers "still waiting" so the agent calls again, staying under the host's tool timeout |

**Channel push.** When you submit, the MCP server attached to that review's agent session sees the
submit in the database and pushes the review into the session as a channel message (Claude Code
channels: the server declares the channel capability and sends a notification). The message holds
every new comment with its file, lines and thread id, and tells the agent to answer each one with
`reply`/`resolve`. This needs checking first (see M0): if channels aren't available or behave
differently, `wait_for_review` covers it inside glos, and the separate Claude Code plugin can wait
for a submit through the CLI and send the review as a prompt.

**Which session gets it:** each review is tied to a folder (the worktree, or the folder holding the
files). The MCP server registers the folder it was started in; a submit goes to the sessions
registered for that folder. If several sessions serve the same folder, the most recently attached
one gets it. Before delivering, sessions whose process is gone (checked by process id) are
removed. With none attached, the review waits and is delivered when one attaches.

## The CLI

For scripts and agents without MCP, every operation exists as a subcommand with JSON output:
`glos review list | show | comments | reply | resolve | annotate | submit`. This is also how the
separate Claude Code plugin and tests drive glos.

## The dashboard

`glos dashboard`, or `space d` inside a review, lists every review on this machine: its source
(repo and branch or worktree, or the two paths), the agent's state (working, waiting for review,
replied), and open, outdated and new-reply counts. `enter` opens one. It's built from the store,
including which agent sessions are attached, and refreshes on its own.

## Launching

glos works in any terminal and never depends on one.

- **Standalone:** `glos` in a repo finds the branch, its base and any existing review;
  `glos old new` reviews two files or folders.
- **Opening itself (optional):** `open_command` in `config.toml` is a shell command glos runs to
  open itself in a new pane, for example `tmux split-window -h glos`,
  `kitten @ launch --location=vsplit glos` (kitty) or `wezterm cli split-pane -- glos`; Ghostty
  through your own script. Without it, glos never tries to open a window.
- **When the agent asks:** `request_review` shows in an open glos, runs `open_command`, or has the
  agent ask you to run `glos`, in that order.
- **`glos open`:** follows the same rule from a shell, so scripts and plugins can launch glos.

## Crates

Picked for being widely used and maintained. Versions are fixed when the project is created.

| Need | Crate |
|---|---|
| Terminal UI | `ratatui`, `crossterm` (mouse, bracketed paste, focus events) |
| Text buffer | `ropey` |
| Motions, selections, text objects | `helix-core` (git dependency at the pinned Helix release) |
| Syntax highlighting | `tree-sitter`, plus Helix's highlighter crate `tree-house` if its API suits; `tree-sitter-highlight` otherwise; `cc` in `build.rs` to compile the grammars |
| Fuzzy matching | `nucleo` (Helix's own matcher) |
| Git (git sources only) | `gix` |
| Line diff | `imara-diff` (used by Helix and gix) |
| Folder walking | `ignore` (honours `.gitignore`) |
| File watching | `notify` with `notify-debouncer-full` |
| Storage | `rusqlite` (bundled SQLite) |
| MCP | `rmcp` (the official Rust SDK) |
| Async | `tokio` (needed by `rmcp`) |
| CLI | `clap` |
| Serialisation | `serde`, `serde_json`, `toml` |
| Paths | `etcetera` (what Helix uses) |
| Ids and time | `uuid`, `jiff` |
| Errors and logs | `anyhow`, `thiserror`, `tracing`, `tracing-subscriber` |
| Tests | `insta` (snapshots of rendered screens and JSON), `tempfile` |

## Milestones

The MVP is M0 through M5: reading and commenting, threads in both directions, all three views and
the dashboard.

- **M0, spikes.** Prove the four risky pieces before building around them:
  1. A Claude Code channel push from an `rmcp` server reaches a session as a message.
  2. A grammar compiled into glos at Helix's pinned revision, with Helix's embedded queries,
     highlights a file through `tree-house` or `tree-sitter-highlight`. Test a language whose
     query inherits (TypeScript inherits from ecmascript) and a theme that uses `inherits`, and
     measure how much the starting set of languages adds to the binary.
  3. gix produces the branch-vs-base diff including uncommitted and untracked files.
  4. `helix-core` at the pinned release builds as a dependency of glos and drives motions and
     selections over a ropey buffer, without pulling in the rest of Helix. If not, copy its
     movement, selection and text-object files instead.
- **M1, the reader.** Store and data model; sources (two files, two folders, git branch vs base);
  full-file + gutter view; Helix keymap engine with motions and selections from `helix-core`; the
  key help panel; the keymap checklist; file picker; themes and highlighting; mouse.
- **M2, commenting.** Comments on selections, review-level comments, inline collapsible threads,
  instant saving, snapshots, `space s` submit through the channel, the CLI.
- **Checkpoint.** Use glos for real reviews instead of tuicr before building M3–M5, and adjust the
  rest of the plan from what that shows. Include a fidelity pass: use glos and Helix side by side
  on the same files, and note every difference in how they feel.
- **M3, two-way.** MCP tools (reply, resolve, annotate, request review, focus), live refresh
  through `notify` and the database, re-anchoring and outdated threads.
- **M4, views and scopes.** Unified and side by side; since last review, per commit, files filter;
  file tree.
- **M5, dashboard and launching.** Multi-review dashboard; `open_command` and `glos open`.
- **After the MVP:** in-place editing with the Helix model (`edit`), remappable keys, and polish.

In-place editing is listed after the MVP because the reviewer is useful without it, and it's the
largest piece of Helix to imitate. Move it earlier if quick fixes turn out to be needed daily.

## Open questions

- **Channels:** how stable Claude Code channels are, whether they need a launch flag, and what they
  allow, decided by the M0 spike.
- **`tree-house`:** whether Helix's highlighter crate is usable outside Helix, or
  `tree-sitter-highlight` is needed instead (M0).
- **Re-anchoring rules:** how far to search for moved text before calling a thread outdated.
- **Branch renames and detached HEAD:** how a review follows a renamed branch, and what a review
  made in detached HEAD is keyed by.
- **A folder with no second copy:** a non-git folder has no old side. glos could snapshot it when
  the review starts and diff against that; decide when someone needs it.
