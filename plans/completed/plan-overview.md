# glos

**Status:** Milestones M0 to M5 built (2026-11-04). This stays the map of glos. Its "After the MVP" list and "Open questions" hold the work that has no plan yet.

A terminal code reviewer for work done by coding agents. It feels like Helix, runs locally, and
talks to agents both ways: you comment, the agent answers on the same line, and the review stays
open across sessions until every thread is settled.

Why it exists: reviewing an agent's branch on GitHub means pushing, opening a PR and switching to a
browser for code you didn't write and don't want to publish yet. `tuicr` gets closer but has no way
for agents to plug in, needs a manual save before quitting, and doesn't feel like Helix. Every
decision behind this plan, with the options considered, is in [DECISIONS.md](../DECISIONS.md).

## Goals

- Review an agent's work without leaving the terminal or pushing anything.
- Work on any two files or folders. Git adds branches, commits and a base, but isn't required.
- Feel like Helix: same modes, keys, pickers, themes and syntax highlighting.
- Agents are first-class: they reply, resolve, pre-annotate their changes, ask for review and can
  suggest a line for you to look at, through MCP.
- Nothing you write is ever lost: every comment, and every draft as you type it, is saved at once.
- Standalone: runs in any terminal, with no terminal-specific integration required.
- Fast: the first screen appears before the diff is fully computed.
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
   there. If not, glos runs your `open-command` (a tmux or kitty split, for example) when you've
   set one; otherwise the agent asks you to run `glos`. In a git repo, glos shows the branch since
   it left `main`, uncommitted and untracked files included. You can also run `glos` yourself in
   any repo, or `glos diff old new` on any two files or folders.
2. The agent may already have left notes on its own changes, such as "why I did this" or "unsure
   here". They're marked with the agent's name and drawn in their own colour.
3. You read the code in full-file view, with changed lines marked in the gutter. `]g` jumps to the
   next change and `gn` to the next file, as in Helix. `space g` opens a fuzzy picker of changed
   files, and `space f` opens any file in the repo, read-only, when you need context.
4. Select lines the Helix way (`x`, `X`, `v`, or a mouse drag), then `space c` to comment. A small
   box opens in insert mode; `esc` then `enter` saves the comment. The draft is saved as you type.
5. `space s` submits, with a verdict: approved, changes requested, or just comments. The agent is
   waiting on glos's `wait_for_review` tool, and the review comes back to it as that tool's answer.
   There's no prompt to paste. Until you submit, the statusline shows how many comments the agent hasn't received.
6. As the agent works, glos refreshes live without moving your view: the diff changes, threads
   follow their lines, and replies appear in place, marked unread. `]u` jumps to the next unread
   reply. A thread whose code is gone is marked outdated.
7. Round 2: switch the scope to "since last review" to see only what changed in response to you,
   with or without git. Reply, resolve, or reopen, then submit again. When you're done, approve and
   close the review.
8. With several agents on several branches or worktrees, the dashboard lists every review with
   unsubmitted comments, unread replies and open threads, and you jump between them.

## What glos can review

- **Two files:** `glos diff old.rs new.rs`. The most basic review; no git needed.
- **Two folders:** `glos diff old/ new/`. Files are matched by relative path; `.gitignore` rules
  are honoured if present.
- **A git repo:** `glos` inside it. Branch vs base by default, plus per-commit scope.
- **Git revisions:** `glos diff HEAD~1` compares a revision with your files (uncommitted work
  included); `glos diff HEAD~3 HEAD~1` compares two revisions. An existing path wins over a
  revision of the same name. Both belong to the branch's review.

**Comments on any diff.** You can comment in every view and scope. Each thread keeps the diff
it was made on: the commits on each side (or "your files") and a sentence saying what was
compared (`commit 2/3 d0c4165 "add b" vs its parent 1a2b3c4`). A thread made on your files
follows them as they change; one made on a commit stays on that commit's code, and is shown in
other views wherever the same code still is, marked with the version it was made on. Agents get
the diff with every comment, the files in the review, and whether the code a comment is about is
still in the files now, has moved, or has changed (outdated).

Each source becomes the same list of changes, and glos diffs text itself with imara-diff. Git is
only one way to get the old side; everything after that is shared.

**Kinds of change.** Besides edited files, glos shows added and deleted files, renames (detected by
gix for git; by identical content for folders), mode changes and symlinks, each as a one-line
header. Binary files get a one-line notice instead of a diff. Lockfiles, minified files and files
marked generated in `.gitattributes` (`linguist-generated`) start collapsed; `enter` expands them.

## Architecture

One Rust binary crate, split into modules. Split modules into crates later only if compile times
or reuse call for it.

| Module | Job |
|---|---|
| `cli` | Entry point and subcommands (see The CLI) |
| `core` | Review data model: reviews, rounds, snapshots, threads, comments, drafts, anchors, and re-anchoring |
| `source` | Turns two files, two folders or a git repo into changes; git through gix |
| `store` | The SQLite database, its migrations, and snapshots |
| `ui` | ratatui front end: views, keymap engine, key help panel, comment box, pickers, popups, themes, highlighting |
| `mcp` | MCP server for agents, including delivering submitted reviews |
| `edit` | After the MVP: in-place editing on top of `helix-core` |

### Processes and how they talk

- **The UI** (`glos`) is the only process that draws.
- **The MCP server** (`glos mcp`) is started by the agent host (Claude Code) over stdio, one per
  agent session. It registers itself and the folder it was started in, and reads and writes review
  state in the store. It's also how your submitted review reaches that agent's session.
- **The CLI** (`glos list`, `glos reply`, …) reads and writes the same state for scripts and for
  agents without MCP.
- All of them share one SQLite database, and that's the only channel between them: no socket and
  no daemon. Each process notices the others' changes by checking `PRAGMA data_version` about twice
  a second. That's how replies appear live in the UI, how an MCP server learns you submitted, and
  how an agent's suggestion reaches an open view.

### Data model (`core`)

```
Review     id, source (two files | two folders | git repo + worktree + branch + base),
           status (open | closed | archived), created/updated, rounds[], threads[]
Round      n, submitted_at, verdict (approved | changes requested | comments), snapshot
Snapshot   the content of every file in the review at that moment: path → content hash
           (git: also the head commit, for reference)
Thread     id, anchor, status (open | resolved | outdated), comments[], created_round,
           last_seen_at (for unread replies)
Anchor     file path, line range on the new side, the anchored lines' text, context lines
           (3 above, 3 below), hash of the file's content when the comment was made;
           or none for a review-level thread
Comment    id, author (you | agent name), kind (comment | annotation), body, created_at,
           edited_at, submitted_in (round, or none while unsubmitted)
Draft      the text in an open comment box and where it belongs, saved as you type
Session    MCP server process id, agent name (from the MCP client's clientInfo), folder it
           serves, last seen
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
the code as it was. This is the riskiest logic in the project and gets the most tests. The same
mapping keeps your cursor and scroll position on the same code when a file changes under you.

While the agent is editing, a thread isn't marked outdated straight away. glos waits until the
files have been quiet for a few seconds, or the round ends, so threads don't flicker between open
and outdated mid-edit. Once outdated, a thread stays outdated until you reopen or resolve it.

Comments go on the new side only. Comments on removed lines may come later, in the unified and
side-by-side views.

**A review's life.** A review is open until you close it (`:close`, usually after approving). glos
archives it on its own when its branch is merged into the base or deleted. Closed and archived
reviews leave the dashboard but stay searchable with `glos list --all`. `glos gc` deletes archived
reviews older than a set age and any snapshot content nothing refers to any more.

### Storage (`store`)

- One SQLite database (`rusqlite`, bundled), `glos.db`. glos uses the XDG folders on macOS and
  Linux alike, as Helix does: data in `$XDG_DATA_HOME/glos` (`~/.local/share/glos`), config in
  `$XDG_CONFIG_HOME/glos` (`~/.config/glos`), logs in `$XDG_STATE_HOME/glos`
  (`~/.local/state/glos`). `--db` or `GLOS_DB` points at another database, for tests and scripts.
- Every action (a comment, a reply, a resolve) is one transaction, written the moment it happens.
  Comment drafts are saved as you type. There's no "save" step, and quitting never loses anything.
- SQLite handles several processes writing at once (WAL mode with a busy timeout), so the UI, MCP
  servers and CLI never overwrite each other's changes.
- The schema is versioned with `PRAGMA user_version`. Migrations run at startup, and the database
  is copied to a backup first.
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
- **Per commit** (git only): step through the branch's commits with `]C` / `[C`.
- **Files filter:** by path or glob, or only files with open threads.

In every view, a changed line has a background tinted toward the theme's addition or removal
colour, and the words that changed within it get a stronger tint, as on GitHub and in `delta`.
In the whole-file view, added and modified lines take the addition tint, since they're the new
text. `plan-diff-highlight.md` has the details.

Navigation: the changed-file picker (`space g`) is primary; `gn` / `gp` step through changed files;
`space f` opens any file in the repo, read-only, for context; `space e` toggles a file tree with
change counts and thread badges. Threads are drawn inline under their lines as virtual text,
collapsed to a one-line marker until you open them, like Helix's inline diagnostics.

**Long lines.** Soft wrap is off by default, as in Helix, with horizontal scrolling;
`:toggle soft-wrap` (and `[editor.soft-wrap]` in config) turns it on. Tab width follows
`languages.toml`. Wide characters and emoji take their real width.

## Writing comments

`space c` opens the comment box under the selection; `space C` opens one for a review-level
comment, and `space r` one for a reply.

- The box is a small Helix-style buffer. It opens in insert mode; `esc` goes to normal mode, where
  `enter` saves the comment and `q` closes the box. Motions and selections work as in the rest of
  glos (`helix-core`).
- The draft is saved as you type. Closing the box, quitting or a crash keeps it, and it comes back
  when you reopen the box on that line.
- `:editor` opens the draft in `$EDITOR` for long comments, and brings the result back.
- Bodies are Markdown, shown as plain text with light styling (code spans, lists).
- `space m` on a thread edits your latest comment in it. Until you submit, edits and deletes (an
  empty save, after a confirmation) are silent. After a submit, an edit is marked as edited and the
  agent is told in the next submit.

**Unsubmitted comments** are always visible: the statusline shows their count (`3 unsent`), quitting
with any reminds you once, and the dashboard lists them per review.

## Submitting

`space s` (or `:submit [approve|changes|comment]`) sends every unsubmitted comment as one round,
with a verdict:

- **Approve:** you're done; the agent may finish up (and you may `:close` the review).
- **Changes requested:** the agent should address the threads and request review again.
- **Comment:** questions or notes, without a verdict.

The verdict goes to the agent with the comments. Submitting with no comments and an approval is
allowed; that's an LGTM.

## Keymap (Helix first)

glos follows Helix's keys wherever an action exists there: `hjkl`, `w`/`b`/`e`, `gg`/`ge`, `gh`/`gl`,
`x`/`X`, `v` (select mode), `%`, `;`, `,`, `/` search, `y`/`space y` (yank), `:` commands,
`space`-mode, and so on. `u` (undo) and the other editing keys come with in-place editing, after
the MVP. Review concepts map onto Helix's: changes are hunks (`]g`), threads are diagnostics (`]d`,
`space d`), changed files are like buffers (`gn`, `gp`). Review actions are added where Helix leaves
room:

| Key | Action |
|---|---|
| `space c` | Comment on the selection (on the line when nothing is selected) |
| `space C` | Review-level comment |
| `space r` | Reply to the thread under the cursor |
| `space m` | Edit (or delete) your latest comment on the thread under the cursor |
| `space x` | Resolve or reopen the thread under the cursor |
| `space s` | Submit the review to the agent, with a verdict |
| `space v` | Switch view |
| `space o` | Switch scope |
| `space g` | Changed-file picker (as in Helix) |
| `space f` | Any file in the repo, read-only (as in Helix) |
| `space d` / `space D` | Thread picker: this file / the whole review (Helix's diagnostic pickers) |
| `space e` | Toggle file tree (Helix's file explorer key) |
| `space l` | Dashboard: the list of reviews |
| `space n` | Jump to the line the agent suggested |
| `space y` | Yank the selection to the system clipboard (as in Helix) |
| `]g` `[g` | Next / previous change (as in Helix) |
| `]d` `[d` | Next / previous thread (Helix's diagnostic motions) |
| `]u` `[u` | Next / previous unread reply |
| `gn` `gp` | Next / previous changed file (Helix's buffer motions) |
| `]C` `[C` | Next / previous commit (per-commit scope) |
| `enter` | Expand or collapse the thread, or collapsed file, under the cursor |
| `i` `a` `o` … | After the MVP: edit in place on the new side (Helix insert mode); changes save on `:w` |

Some of these keys mean something else in Helix: `space c` and `space C` toggle comments, `space s`
is the symbol picker and `space r` renames. Those need an LSP or editing, which glos doesn't have,
so glos reuses the keys for review actions on purpose. Helix's tree-sitter motions (`]f` function,
`]t` type, `]c` comment, `]a` argument) keep their Helix meaning. Every key above is checked
against the keymap checklist at the pinned Helix release.

Every action is also a typable command (`:comment`, `:reply`, `:resolve`, `:submit`, `:close`,
`:view`, `:scope`, `:dashboard`, `:editor`), with completion. Keys can be remapped in
`config.toml`, as in Helix.

**The key help panel** is essential, not an extra. As in Helix's infobox, pressing any key that
starts a sequence (`space`, `g`, `]`, `[`, `m`, `z`, …) opens a panel listing the keys that can
follow, each with a one-line description. glos goes one step further and lists only what applies
where you are: thread actions when the cursor is on a thread, `]C`/`[C` only in per-commit scope,
submit only when there's something to submit. The panel is built from the live keymap, so remapped
keys show their new bindings. It's part of the keymap engine in M1.

**`Ctrl-G` shows every key for where you are,** wherever you are: the file view, the comment box
(insert or normal mode), the file tree, a picker, the `:` line. It opens Helix's command palette
(also `space ?` in normal mode), listing only what applies there; type to filter, `ret` runs a
command, `esc` or `Ctrl-G` goes back. `Ctrl-G` is free in every Helix mode, and every terminal
sends it the same way (unlike `Ctrl-/`, or `Option` keys on a Mac).

**Mouse:** on by default, and `[editor] mouse = false` turns it off, as in Helix. Click to move the
cursor, drag to select, wheel to scroll; click files in the tree and picker, click a thread marker
to expand it, and click buttons in popups. Clicking the tree or the code focuses it, and the
wheel scrolls whatever is under the pointer, including sideways ([plan-mouse.md](plan-mouse.md)). While glos captures the mouse, the terminal's own text
selection needs a modifier (usually shift), so `space y` yanks to the system clipboard: `pbcopy`,
`wl-copy` or `xclip` as Helix does, or OSC 52 over SSH. The keyboard stays primary.

## The terminal

- **Untrusted text.** Agent comments and file contents can contain escape sequences that change
  the window title, write to your clipboard or corrupt the screen. glos strips control characters
  from everything it draws, and shows them as visible symbols in code, as Helix does.
- **Logs never go to the terminal.** In `glos mcp`, stdout carries the MCP protocol and one stray
  print breaks it; in the UI, output corrupts the screen. Logs go to `~/.local/state/glos/glos.log`;
  `-v` adds detail.
- **Behaving well:** the terminal is restored on a panic, `ctrl-z` suspends and resumes, resizing
  redraws, and focus events trigger a refresh.
- **Colour.** Themes are 24-bit; on terminals without truecolor (checked through `COLORTERM`, as
  Helix does) colours are mapped to the nearest 256-colour ones. `NO_COLOR` is respected. Authors
  are never told apart by colour alone: every comment carries its author's name.
- **Your view stays put.** Live refreshes and agent activity never move the cursor or scroll. An
  agent's `suggest` shows as a notice ("agent suggests src/main.rs:42"); `space n` jumps there.
- **Speed.** The first screen draws before the whole diff is computed; files are diffed and
  highlighted as they're needed, and only visible lines are highlighted. Target: under 100 ms to
  the first screen on a typical branch.

## Helix fidelity

Everything glos takes from Helix is compiled into the binary from a pinned Helix release
(25.07.1). glos
never reads Helix's files or config at run time, and works the same whether Helix is installed or
not.

- **Themes and config:** Helix's themes are embedded. glos's own `config.toml` sits in
  `~/.config/glos/` and mirrors Helix's layout (`theme`, `[keys.normal]`, `[editor]`). Your own
  themes go in `~/.config/glos/themes/` in Helix's format, so a Helix theme can be copied over as
  is. Themes that build on another (`inherits`) and named colours (`[palette]`) are resolved as
  Helix does. A config error is shown at startup, and glos carries on with the defaults, as Helix
  does.
- **Syntax highlighting:** tree-sitter, with the grammars compiled into glos. `cargo xtask fetch-helix` takes from a
  pinned Helix release its `languages.toml` (which maps files to languages and pins each grammar's
  source and revision), the highlight queries and the themes. It fetches each grammar's source at
  the revision Helix pins and vendors it in the repo (committed: about 5 MB of git history per
  Helix upgrade, and builds never fetch). All of it lives in the `crates/glos-helix` crate
  (MPL-2.0), the only part of glos that depends on Helix's crates; glos imports what it needs
  from there. Its `build.rs` compiles the grammars with `cc` and links them
  statically into `tree-house`, Helix's highlighter; the queries, themes and `languages.toml` are
  embedded with `include_str!`. Each language's queries are compiled the first time it's needed,
  off the UI thread: compiling all of them takes about 260 ms, too slow for the first screen. Using Helix's
  pinned revisions keeps queries and grammars matching. Queries that build on another language's
  (`; inherits:`) are followed as Helix does. Moving to a new Helix release means rerunning the
  task.
- **Languages:** a starting set, listed in one file: Rust, TypeScript/TSX, JavaScript, Python, Go,
  C, C++, Java, Ruby, Bash, JSON, TOML, YAML, Markdown, HTML, CSS, SQL and Lua. Adding one is a
  line in `xtask/src/fetch_helix.rs` plus a rerun of the task. Files in other languages show as plain text.
- **Licences:** Helix's queries, themes and `languages.toml` are MPL-2.0 and stay under it, with
  their notice, in their own folder. Grammars keep their own licences (mostly MIT).
- **Movement and selections come from Helix itself.** `helix-core` (plain functions over a ropey
  buffer, with no UI) provides motions, selections, multiple cursors and text objects. It's a git
  dependency pinned to the same Helix release as the grammars, with glos's `Cargo.lock` seeded
  from Helix's at that tag. The keymap layer (counts, modes, `x`, `f`/`t`, `m`) lives in
  helix-term, not helix-core, so its thin wrappers are ported into one MPL-2.0 file. glos never
  rewrites this behaviour, so `w`, `x`, `%`, `mi(` and counts behave exactly as in Helix, and
  Helix's own unit tests come with them.
- **Editing (after the MVP):** the same `helix-core` model with `insert`/`normal`/`select` modes.
  Writes go to the file on the new side, which triggers a live refresh like any other change.
- **The keymap checklist.** `cargo xtask keymap` generates `../KEYMAP.md` from Helix's default keymap at the
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

`glos mcp` speaks MCP over stdio (crate `rmcp`). glos never works out what to review: the agent
names it with `open_review` and passes the review id it gets to every other tool. Tools:

| Tool | What it does |
|---|---|
| `open_review` | Name what to review, exactly: a git branch (the worktree folder that has it checked out, the branch, and its base) or two files or folders, by absolute paths. Answers with the review's id, made if there's none yet. The user isn't told |
| `get_review` | A summary of the review: verdict, scope, files, and the open threads with what's new since the agent's last call. Full threads on request, by id, so the agent's context isn't flooded |
| `reply` | Reply on a thread |
| `resolve` | Resolve a thread with a note; `reopen` reverses it |
| `annotate` | Leave an agent note on a line or range before you review |
| `request_review` | Say the work is ready, optionally scoped to the files it touched. An open glos shows the request; otherwise glos runs `open-command` if set, and the answer tells the agent to ask you to run `glos` if nothing opened |
| `suggest` | Suggest a file and line for you to look at. It shows as a notice in your open view, and `space n` jumps there; your view never moves on its own |
| `wait_for_review` | How a submitted review reaches the agent: waits a few minutes for you to submit, then answers with the review, or "still waiting" so the agent calls again, staying under the host's tool timeout |

**Delivering a review.** After `request_review`, the agent calls `wait_for_review`. When you
submit, the MCP server waiting on that review's id sees the submit in the database and answers
the waiting call. The answer holds the verdict and every new or edited comment with its file,
lines and thread id, and tells the agent to answer each one with `reply`/`resolve`, then call
`request_review` again (or finish, on an approval). This works in any MCP host, with no flags or
organisation settings. While it waits, the session is busy; that's the cost.

The separate Claude Code plugin can add a `Stop` hook that waits for a submit through the CLI and
hands the review back as the agent's next instruction, so the agent needn't loop.

**Channels, later.** Claude Code channels would let the server push the review into an idle
session instead. M0 showed the server side works (the `claude/channel` experimental capability,
`notifications/claude/channel`, and the MCP protocol capped at 2025-11-25), but channels are a
research preview: they need a launch flag, an organisation setting, and drop messages silently
when either is missing. glos supports them once they're no longer experimental; the spike in
`spikes/channel/` (deleted; in git history before fbdf711) is the starting point.

**Which session gets it:** an MCP server over stdio isn't told the host's session id, so each
server is identified by its own process id and named after the `clientInfo` its MCP client sends.
A submit goes to the first `wait_for_review` on that review's id; with none waiting, it waits and
is delivered to the next one. Each server also registers the folder it was started in, which the
dashboard uses to show which agents are around a review; it never decides what's reviewed.

**Setting it up:** `claude mcp add glos -- glos mcp` registers glos with Claude Code; `glos doctor`
checks that it worked.

## The CLI

Plain `glos` reviews the current repo; everything else is a subcommand, so a file named like a
subcommand is never ambiguous:

| Command | What it does |
|---|---|
| `glos` | Open the review for the current repo (branch vs base) |
| `glos diff <old> <new>` | Review two files or two folders, like `git diff --no-index`; or `glos diff <rev> [<rev>]`, like `git diff` |
| `glos list [--all]` | Reviews on this machine: open ones by default, closed and archived with `--all` |
| `glos open-review git <folder> <branch> <base>`, `glos open-review diff <old> <new>` | `open_review` for agents without MCP: prints the review's id |
| `glos show --review <id>` | A review's threads and comments |
| `glos reply`, `resolve`, `reopen`, `annotate`, `submit`, `close` | The same actions as in the UI and MCP, each with `--review <id>`: none guesses the review from the folder |
| `glos dashboard` | Open the dashboard |
| `glos open --review <id>` | Open glos the way `request_review` does (see Launching) |
| `glos mcp` | Run the MCP server over stdio |
| `glos gc` | Delete old archived reviews and unused snapshot content |
| `glos doctor` | Like `hx --health`: database, config, theme, languages, terminal colour support, MCP setup |
| `glos close --review <id>` | Close a review you're done with |
| `glos --review <id>` | Open a review by its id, wherever it is |
| `glos completions <shell>` | Shell completions (`clap_complete`); `glos man` prints the man page (`clap_mangen`) |

- **Output:** readable text by default; `--json` for scripts and agents, as `gh` does. Every JSON
  document carries a `"version"` field, and its shape only changes with that version.
- **Exit codes:** `0` success, `1` error, `2` bad usage (clap's default). `glos show` and
  `glos list` exit `0` with empty results.
- **No changes:** `glos` in a repo with nothing to review says so and exits `0` instead of opening
  an empty screen. Outside a repo with no arguments, it explains `glos diff`.

This is also how the separate Claude Code plugin and tests drive glos.

## The dashboard

`glos dashboard`, or `space l` inside a review, lists every open review on this machine: its
source (repo and branch or worktree, or the two paths), the agent's state (working, waiting for
review, replied), and unsubmitted, unread, open and outdated counts. `enter` opens one. It's built
from the store, including which agent sessions are attached, and refreshes on its own.

## Launching

glos works in any terminal and never depends on one.

- **Standalone:** `glos` in a repo finds the branch, its base and any existing review;
  `glos diff old new` reviews two files or folders.
- **Opening itself (optional):** `open-command` in `config.toml` is the start of a shell command
  that runs the rest in a new pane, for example `tmux split-window -h`,
  `kitten @ launch --location=vsplit` (kitty) or `wezterm cli split-pane --`; Ghostty through
  your own script. glos adds its own command line (`glos --db … --review <id>`). Without it,
  glos never tries to open a window.
- **When the agent asks:** `request_review` shows in an open glos, runs `open-command`, or has the
  agent ask you to run `glos`, in that order.
- **`glos open`:** follows the same rule from a shell, so scripts and plugins can launch glos.

## Crates

Picked for being widely used and maintained. Versions are fixed when the project is created.

| Need | Crate |
|---|---|
| Terminal UI | `ratatui`, `crossterm` (mouse, bracketed paste, focus events) |
| Text buffer | `ropey` |
| Motions, selections, text objects | `helix-core` (git dependency at the pinned Helix release) |
| Syntax highlighting | `tree-house` 0.3.0 (Helix's highlighter, the version helix-core uses, so one copy of tree-sitter), with `tree-house-bindings`' `tree-sitter-language` feature for statically linked grammars; `cc` in `build.rs` to compile them |
| Fuzzy matching | `nucleo` (Helix's own matcher) |
| Git (git sources only) | `gix` |
| Line diff | `imara-diff` (used by Helix and gix) |
| Folder walking | `ignore` (honours `.gitignore`) |
| File watching | `notify` with `notify-debouncer-full` |
| Storage | `rusqlite` (bundled SQLite) |
| MCP | `rmcp` (the official Rust SDK) |
| Async | `tokio` (needed by `rmcp`) |
| CLI | `clap`, `clap_complete`, `clap_mangen` |
| Serialisation | `serde`, `serde_json`, `toml` |
| Paths | `etcetera` (what Helix uses), with its XDG strategy on both systems |
| Ids and time | `uuid`, `jiff` |
| Errors and logs | `anyhow`, `thiserror`, `tracing`, `tracing-subscriber` (to the log file) |
| Tests | `insta` (snapshots of rendered screens and JSON), `tempfile` |

## Testing

Tests are written with each feature, covering ordinary use as well as the cases likely to go
wrong. Each commit includes its tests, and `cargo fmt`, `cargo clippy` (no warnings) and
`cargo test` pass before every commit.

- **Commands** are tested in Helix's selection notation (`#[h|]#ello`). Where a test and
  helix-core disagree, helix-core wins: glos copies Helix, it doesn't reinterpret it.
- **The app** is tested by feeding it keys and mouse events (`App` runs without a terminal), then
  checking the selection, the view, the status and the screen.
- **Screens** that must look like Helix are `insta` snapshots, with separate checks of their
  colours, which text snapshots don't see.
- **Sources** are tested on real git repos and folders built in a temporary directory.
- **Re-anchoring** (M3) gets the most tests of all.

## Milestones

The MVP is M0 through M5: reading and commenting, threads in both directions, all three views and
the dashboard.

- **M0, spikes (done; the spikes and their `FINDINGS.md` are in git history before fbdf711).** Prove the four risky pieces before building
  around them. All four work. The channel push reaches the real Claude Code handshake, but
  delivery is unproven: this Team account's org policy blocks channels.
  1. A Claude Code channel push from an `rmcp` server reaches a session as a message.
  2. A grammar compiled into glos at Helix's pinned revision, with Helix's embedded queries,
     highlights a file through `tree-house` or `tree-sitter-highlight`. Test a language whose
     query inherits (TypeScript inherits from ecmascript) and a theme that uses `inherits`, and
     measure how much the starting set of languages adds to the binary.
  3. gix produces the branch-vs-base diff including uncommitted and untracked files.
  4. `helix-core` at the pinned release builds as a dependency of glos and drives motions and
     selections over a ropey buffer, without pulling in the rest of Helix. If not, copy its
     movement, selection and text-object files instead.
- **M1, the reader.** Store with versioned schema and data model; sources (two files, two folders,
  git branch vs base) and kinds of change; full-file + gutter view; Helix keymap engine with
  motions and selections from `helix-core`; the key help panel; the keymap checklist; changed-file
  and any-file pickers; themes and highlighting; soft wrap and horizontal scrolling; mouse and
  clipboard; the terminal basics (control characters stripped, logs to a file, panic restore,
  suspend, colour fallback); `glos doctor`.
  *Status (2026-11-06):* built, except selecting by syntax node (`A-o`, `A-i`), `gw`, the
  jumplist picker (`space j`) and global search (`space /`). The tree-sitter text objects and
  motions (`mif`, `]f`…) came with folding (`plan-folding.md`). `../KEYMAP.md` tracks the keys. The
  first screen draws in about 50 ms on this repo.
- **M2, commenting.** The comment box with drafts saved as you type and `:editor`; comments on
  selections, review-level comments, editing and deleting; inline collapsible threads; the
  unsubmitted count; snapshots; `space s` submit with a verdict; `glos mcp` with `get_review`,
  `request_review` and `wait_for_review`, so a submit reaches the agent; the CLI with `--json`.
  *Status (2026-11-04):* built: the comment box (`space c`, `space C`, `space r`, `space m`) with
  drafts saved per keystroke and `:editor`; threads inline, collapsed to one line, `ret` or a click
  opening them; `]d`/`[d` and the thread pickers (`space d`, `space D`); the unsent count and the
  quit reminder; `space s` and `:submit` with snapshots; `:` completion; the key help panel showing
  only what applies; `glos mcp` (sessions registered by process id and agent name); `glos list`,
  `show` and `submit` with `--json`. An agent's review request shows in an open glos within half
  a second. Resolve and the rest of the agent tools stay in M3.
- **Checkpoint.** Use glos for real reviews instead of tuicr before building M3–M5, and adjust the
  rest of the plan from what that shows. Include a fidelity pass: use glos and Helix side by side
  on the same files, and note every difference in how they feel.
- **M3, two-way.** The rest of the MCP tools (reply, resolve, annotate, suggest), live refresh
  through `notify` and the database without moving your view, unread replies, re-anchoring and
  outdated threads.
  *Status (2026-11-04):* built: the MCP tools `reply`, `resolve`, `reopen`, `annotate` and
  `suggest` (and `glos reply`, `resolve`, `reopen`, `annotate` with `--as`); agents' replies and
  notes appear live, marked unread, with `]u`/`[u` and an unread count; `space x` resolves or
  reopens; `space n` goes to a suggestion; files are watched (`notify`, `.git` and the root
  `.gitignore` left out) and read again on change or focus, keeping cursor and scroll on the same
  code through Helix's `compare_ropes`; threads re-anchor on every change and become outdated
  only after 3 quiet seconds or at a submit, shown with the code they were on.
- **M4, views and scopes.** Unified and side by side; since last review, per commit, files filter;
  file tree.
  *Status (2026-11-04):* built: `space v` (`f` file, `u` unified, `s` side by side) and `:view`,
  keeping your line; the unified and side-by-side views are read-only documents made of both
  sides' lines, so motions, search and selections work in them and comments map to the new side;
  `space o` (`b` base, `r` since your last review, `c` per commit with `]C`/`[C`, `f` filter) and
  `:scope`, `:filter <glob>` / `:filter threads`; `space e` file tree with `+added -removed` and
  open-thread counts. Removed lines shown inline in the file view are left for later.
- **M5, dashboard and launching.** Multi-review dashboard; closing and archiving reviews, `glos gc`;
  `open_command` and `glos open`; shell completions and the man page.
  *Status (2026-11-04):* built: the dashboard (`space l`, `:dashboard`, `glos dashboard`) as a
  picker of open reviews with the agent's state and counts, refreshing on its own, `ret` switching
  to a review; `:close` and `glos close`; reviews archived when their folder or branch is gone or
  the branch was merged; `glos gc [--days N]`; `open-command` (a prefix glos adds itself to),
  `glos open` and `glos --review <id>`, used by `request_review` unless a glos already shows the
  review; `glos completions <shell>` and `glos man`.
- **After the MVP:** in-place editing with the Helix model (`edit`), remappable keys, and polish;
  channel push once Claude Code channels are no longer experimental.

In-place editing is listed after the MVP because the reviewer is useful without it, and it's the
largest piece of Helix to imitate. Move it earlier if quick fixes turn out to be needed daily.

## Open questions

- **Re-anchoring rules:** how far to search for moved text before calling a thread outdated
  (M3 searches the whole file, nearest first), and whether a small edit inside the anchored
  lines (a typo fixed) should keep the thread rather than outdate it (M3 matches exact text only).
- **Branch renames and detached HEAD:** how a review follows a renamed branch, and what a review
  made in detached HEAD is keyed by.
- **The default theme:** Helix's `default` is low contrast, and collapsed thread markers (drawn in
  its `info` colour) are hard to read in it. Choose a higher-contrast built-in theme as glos's
  default, and check thread rows, the comment box and the gutter marks in it.
- **A folder with no second copy:** a non-git folder has no old side. glos could snapshot it when
  the review starts and diff against that; decide when someone needs it.
