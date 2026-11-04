# Decisions

Each decision made while planning glos on 2026-11-03, with revisions from a review of the plan on
2026-11-04: the options considered, what was chosen, and why. The design itself is in
[PLAN.md](PLAN.md).

## Why build it at all

- **Considered:** a Claude Code pane mod; Plannotator, Preflight or Diffsmith (browser or desktop
  apps); tuicr in a Ghostty split, which was tried and wired up to Claude Code.
- **Chosen:** a dedicated terminal tool.
- **Why:** the pane mod is capped at 45% of the terminal width in fullscreen and can't fit long
  lines. Browser and desktop tools leave the terminal. tuicr works, but has no hooks for agents,
  loses comments unless you save before quitting, and its interface isn't Helix.

## Agent integration

- **Options:** MCP server; event hooks; CLI + JSON; local socket/RPC.
- **Chosen:** MCP server and CLI + JSON.
- **Why:** MCP lets any agent use glos natively; the CLI serves scripts and agents without MCP.
- **Revised 2026-11-04:** the local socket was dropped. The shared SQLite database (see Storage)
  already carries changes between processes, and every process notices them by checking
  `PRAGMA data_version`, so replies still appear live in an open UI.

## Delivering a submitted review

- **Options:** MCP channel push; a Claude Code mod that submits a prompt; the agent polls
  (`wait_for_review`).
- **Chosen:** MCP channel push.
- **Why:** no mod is needed, and the review arrives as you submit. The other two remain as fallbacks
  (`wait_for_review` is in the tool list) pending the M0 spike.
- **Revised 2026-11-04, after M0:** `wait_for_review` is the baseline. Channels are a research
  preview: they need a launch flag and an organisation setting (blocked on the account used for
  the spike) and drop messages silently otherwise. glos supports them once they're no longer
  experimental. The Claude Code plugin can add a `Stop` hook so the agent needn't loop.

## Helix fidelity

- **Options:** keymap and modes; pickers and popups; themes and config; tree-sitter.
- **Chosen:** all four.

## Foundation

- **Options:** ratatui with Helix-style behaviour; Helix's crates as git dependencies; a fork of
  Helix.
- **Chosen:** ratatui with Helix-style behaviour.
- **Why:** stable crates.io dependencies. Helix's crates have no stable API and pull in the whole
  editor, and a fork means maintaining a large editor.
- **Revised 2026-11-04:** one exception, `helix-core`, for motions, selections and text objects.
  Rewriting them from memory is how a near-copy ends up feeling off. Pinning it to one Helix
  release avoids the unstable API, and `helix-core` alone has no UI and isn't the whole editor.
  The M0 spike checks that; the fallback is copying its files in under MPL-2.0.
- **Making the feel checkable (2026-11-04):** a generated keymap checklist (`KEYMAP.md`), tests in
  Helix's selection notation, screen snapshots of the elements that must look like Helix, and a
  side-by-side fidelity pass at the M2 checkpoint.

## Code layout (2026-11-04)

- **Options:** a Cargo workspace of eight focused crates; one crate split into modules.
- **Chosen:** one crate split into modules.
- **Why:** the crate boundaries would be guesses before any code exists. Split later if compile
  times or reuse call for it.

## Scope: review only, or editing too

- **Options:** review only; review plus light edits; review plus suggested changes.
- **Chosen:** review plus light edits, done in place with the Helix model (not by handing off to
  Helix).
- **Note:** scheduled after the MVP (see PLAN.md, Milestones).

## Storage

- **Options:** in the repo, git-ignored; in git (notes or refs); the user data dir.
- **Chosen:** the user data dir.
- **Why:** keeps repos clean. Every change is saved at once, as a direct answer to tuicr losing
  unsaved comments.
- **Format, revised 2026-11-04:** a JSON file per review with a file lock was replaced by one
  SQLite database. The UI, MCP servers and CLI all write at once; rewriting a whole JSON file lets
  one overwrite another's changes, and a socket alone needs a process that's always running to
  save anything. SQLite transactions avoid both, with no daemon.
- **Pure-Rust databases considered:** Turso (Limbo), `redb`, `fjall`, `sled`, GlueSQL. Rejected:
  none supports several processes opening the database at once, so each would need a daemon. Speed
  doesn't matter at glos's data size. `rusqlite` with `bundled` needs only a C compiler at build
  time.
- **Snapshots (2026-11-04):** what you reviewed in each round is stored in this database too, not as
  git objects, which `git gc` deletes when nothing references them and which don't exist without
  git.

## Diff views

- **Options:** unified with a split toggle; side by side with a toggle; full file with a gutter.
- **Chosen:** all three, full file + gutter by default, and switching must be easy (`space v`).

## Threads

- **Options:** resolve and reopen; outdated tracking; comment types; review-level comments.
- **Chosen:** resolve and reopen, outdated tracking, review-level comments.
- **Not chosen:** comment types (issue, nit, praise…).
- **Revised 2026-11-04:** comments go on the new side only in the MVP. A thread becomes outdated
  only once the files have been quiet for a few seconds or the round ends, and then stays outdated
  until you reopen or resolve it, so threads don't flicker while the agent edits.

## While the agent works

- **Options:** live refresh; refresh on demand; snapshot rounds.
- **Chosen:** live refresh. Rounds still exist in the data model, to support the
  "since last review" scope.

## Git library

- **Options:** gix; git2; shelling out to git.
- **Chosen:** gix. It's pure Rust and what Helix uses for its diff gutter.

## Git required or not (2026-11-04)

- **Options:** git repos only; git optional.
- **Chosen:** git optional. glos also reviews two files or two folders (`glos diff old new`). Every
  source becomes old/new text that glos diffs itself with imara-diff; gix is used only to read git
  sources. Per-commit scope is git only; since last review works everywhere.

## Review keys

- **Options:** space-mode menu; bracket motions; typable commands; selection-first comments.
- **Chosen:** all four.
- **Key help panel (2026-11-04):** essential, part of M1. As in Helix's infobox, every key that
  starts a sequence opens a panel of the keys that can follow. glos lists only the actions that
  apply where you are, and builds the panel from the live keymap so remapped keys show their new
  bindings.

## What agents can do

- **Options:** reply and resolve; pre-annotate their changes; request review; point you to a line.
- **Chosen:** all four. Pointing you to a line works only with your permission, per review.
- **Revised 2026-11-04:** pointing becomes `suggest`. It shows a notice and `space n` jumps there;
  the agent never moves your view, so no permission is needed.

## Launching

- **Options:** a standalone command; from Claude Code; when the agent asks.
- **Chosen:** all three.
- **Revised 2026-11-04:** no dependency on Ghostty or any terminal. Opening a split is an optional
  `open_command` in `config.toml` (tmux, kitty, WezTerm, or a Ghostty script). Without it, an open
  glos shows the request, or the agent asks you to run `glos`.
- **Revised 2026-11-04:** glos is the CLI and the MCP server only. Launching from Claude Code (a
  `/glos` slash command, replacing `/tuicr`) and the channel fallback mod move to a separate Claude
  Code plugin built on them.

## What a review covers

- **Options:** branch vs base; since last review; per commit; a files filter.
- **Chosen:** all four, as ways to use the same review; branch vs base is the default.

## Several agents at once

- **Options:** one review per branch; a multi-review dashboard; single for now.
- **Chosen:** a multi-review dashboard, in the MVP.

## File navigation

- **Options:** a picker plus a toggleable tree; picker only; an always-on file list.
- **Chosen:** a picker plus a toggleable tree.

## Showing threads

- **Options:** inline and collapsible; a gutter mark with a popup; a side panel.
- **Chosen:** inline and collapsible, like Helix's inline diagnostics.

## Mouse

- **Options:** full; basic; none.
- **Chosen:** full: click, drag to select, scroll, and click files, threads and popup buttons.

## Platforms

- **Options:** macOS and Linux; macOS only; all three.
- **Chosen:** macOS and Linux.

## Grammars and themes

- **Options:** reuse Helix's runtime; bundle common grammars; both.
- **First chosen:** reuse Helix's runtime at run time. Exactly your editor's colours; without
  Helix, plain text.
- **Revised 2026-11-04:** bundle instead. Everything taken from Helix (grammars, queries, themes,
  `languages.toml`) is compiled into glos from a pinned Helix release, so glos runs the same
  whether Helix is installed or not, and a Helix upgrade can't break it. glos no longer reads
  `~/.config/helix`; your own themes go in `~/.config/glos/themes/`.

## MVP

- **Options:** reading and commenting; threads both ways; all three views; the dashboard.
- **Chosen:** all four. In-place editing follows the MVP.
- **Checkpoint (2026-11-04):** after M2, glos is used for real reviews instead of tuicr before M3–M5
  are built, and the plan is adjusted from that.

## UX review (2026-11-04)

A review of the plan for terminal UX and CLI conventions led to these decisions, all now in
PLAN.md:

- **Writing comments:** a small Helix-style box (insert mode first, `esc` then `enter` saves),
  drafts saved as you type, `:editor` for `$EDITOR`. Your comments can be edited and deleted;
  after a submit, edits are marked.
- **Submitting:** every submit carries a verdict (approve, changes requested, comment). The count
  of unsubmitted comments is always visible.
- **Unread replies** are tracked per thread (`]u`).
- **Any file** in the repo can be opened read-only (`space f`) for context.
- **A review's life:** open, closed (`:close`) or archived (automatically when the branch is merged
  or deleted); `glos gc` cleans up.
- **Keys follow Helix's concepts:** changes are hunks (`]g`, `space g`), threads are diagnostics
  (`]d`, `space d`), changed files are like buffers (`gn`, `gp`). This replaces `]h`, `]t`, `]f`,
  `]c` and the `space f` changed-file picker, which contradicted Helix. Commits move to `]C`, the
  dashboard to `space l`.
- **Mouse** can be turned off, and `space y` yanks to the clipboard (OSC 52 over SSH).
- **The terminal:** control characters are stripped from everything drawn; logs go to a file,
  never to stdout or the screen; panic restore, `ctrl-z`, `NO_COLOR` and a 256-colour fallback;
  soft wrap off by default with horizontal scrolling; live refresh never moves your view.
- **Kinds of change:** binary, renamed, deleted, mode and symlink changes are shown; generated
  files and lockfiles start collapsed. The first screen draws before the whole diff is computed.
- **CLI:** `glos diff A B` for files and folders, so paths can't collide with subcommands; flat
  subcommands (`glos list`, not `glos review list`); readable output by default and versioned
  `--json`; documented exit codes; `glos doctor`; completions and a man page.
- **Paths:** XDG folders on macOS too, as Helix does, plus `--db` / `GLOS_DB`. The schema is
  versioned with `PRAGMA user_version`, with a backup before each migration.
- **Agents:** `get_review` returns a summary plus what's new, with full threads on request.
  Sessions are identified by the MCP server's process id and the client's `clientInfo` name.

## Name

- **First pick:** loupe (a jeweller's magnifier), dropped because it clashes with GNOME's image
  viewer (its command is `loupe` on many Linux desktops) and an existing crate.
- **Then:** about 100 names were checked against crates.io, Homebrew, Linux distro packages
  (Repology), commands on the PATH and GitHub. Long names (kibitzer, obelus, margent, scholion)
  were rejected as too long; most short words (stet, gloss, lgtm, ptal, nits, quirk) were taken.
- **Chosen:** glos, from *gloss*: a note written in the margin of a text. Four letters; free on
  crates.io, Homebrew and Linux distros, with no notable GitHub project of that name (checked
  2026-11-03). Trademarks and domains were not checked.

## M0 spikes (2026-11-04)

- **Helix release:** pinned to 25.07.1, the newest.
- **Highlighting:** `tree-house` 0.3.0, not `tree-sitter-highlight`. It's the version helix-core
  already links, so there's one tree-sitter runtime, and it takes statically linked grammars.
- **Vendored grammars:** committed, not fetched at build time. Costs about 5 MB of git history per
  Helix upgrade; builds never need the network.
- **helix-core:** kept as a git dependency, not copied in. It brings only helix-loader,
  helix-stdx and helix-parsec. The keymap wrappers from helix-term are ported into one MPL file.
- **gix:** one `status()` pass with the merge-base as its head tree covers committed, staged,
  unstaged and untracked changes and renames. No fallback to the git command needed.
- **Channels:** declared through rmcp's experimental capabilities; the protocol is capped at
  2025-11-25 so pushes can be delivered. Needs a launch flag while channels are in preview.

## M1 (2026-11-04)

- **tree-house-bindings 0.2.4, not Helix's 0.2.1:** 0.2.1 can't take a statically linked grammar.
  It's still the one tree-sitter runtime, under tree-house 0.3.0.
- **Language loading:** helix-core's `Loader` only loads grammars from disk, so glos has its own,
  parsing `languages.toml` with Helix's types and matching files as helix-core does. Only languages
  with vendored queries count as supported.
- **Drawing goes through helix-core's `DocumentFormatter`,** and scrolling and clicks through its
  visual-offset helpers, so tabs, wide characters and soft wrap match Helix exactly.
- **Commands ported from helix-term** live in one MPL-2.0 file, `src/ui/commands.rs` (now `crates/glos-helix/src/commands.rs`), tested in
  Helix's selection notation. Where a test and helix-core disagreed, helix-core won.

## M2 (2026-11-04)

- **Delivery is a claim in the store:** each round records when an agent took it and which one
  (schema v2). `wait_for_review` takes the oldest round nobody has, in one transaction, so a
  round reaches exactly one waiting agent, and a round submitted while none waits is kept until
  one asks.
- **Agents see only what's sent:** `get_review` and deliveries show submitted comments and agents'
  own; unsent comments and drafts never leave glos.
- **Editing a sent comment** marks it edited and makes it unsent again, so the next round carries
  it. A sent comment can't be deleted, only edited.
- **The comment box is a `File`** like any other, so helix-core's motions, the keymap and search
  work in it unchanged. It's docked under the file view rather than floating at the selection,
  so the commented lines stay visible above it.
- **Threads are helix-core line annotations,** so scrolling and clicks count their rows exactly as
  they're drawn.
- **`space s` is a menu** (`a` approve, `r` request changes, `c` comment), so the verdicts appear in
  the key help panel, which hides the ones that don't apply.
- **Ids** are the first 12 hex digits of a v4 UUID: short enough for agents to quote back. Content
  is hashed as a git blob (gix), so no hashing crate is needed.
- **`glos mcp` serves the review of its starting folder:** the open review whose folder holds it,
  the deepest one. In a git repo with no review yet, `request_review` makes one.

## M3 (2026-11-04)

- **Re-anchoring** maps the anchor through the line diff when the diff doesn't reach into the
  anchored lines (they only moved); otherwise it searches for the exact anchored text, nearest to
  where it should be, with the context lines breaking ties. A successful placement stores the
  file's new content as the thread's base, so the next change maps from there.
- **Outdated waits for quiet:** a thread whose text is gone is "pending" until the files have
  been quiet for 3 seconds, or you submit. Resolved threads whose code went stay resolved;
  outdated ones stay outdated until reopened.
- **Your view through a reload** uses helix-core's `compare_ropes`, the same mapping Helix uses
  when a file changes on disk. A file that's no longer changed stays on screen as context.
- **Agent actions share one implementation** (`mcp::Agent`) between the MCP tools and the CLI,
  so the checks match: agents can only touch threads they can see, paths can't leave the
  review's folder, and resolving needs a note. Over MCP a refusal is the tool's answer; the CLI
  exits 1.
- **Unread** is per thread: agents' comments after you last opened it. Opening a thread (`ret`, a
  click, `]u`, the picker) reads it.
- **One suggestion per review,** the latest, kept on the review row: a suggestion is a pointer,
  not history.

## M4 (2026-11-04)

- **Views are documents:** the unified and side-by-side views are built as read-only text, each
  line knowing where it came from (same, added, removed, folded, padding). Helix's motions,
  search, selections and highlighting work on them unchanged; comments take the selected lines
  that are on the new side; threads sit under their new-side line. Highlighting comes from each
  side's own syntax tree, so it's the same as in the file view.
- **Side by side never wraps,** so both sides stay line for line; padding rows keep changes level,
  and thread rows on the right have blank rows on the left.
- **Since last review** compares each file with its snapshot from the latest round, and counts
  files reverted since; it works with or without git.
- **The file tree is a panel,** not a picker: it stays visible while you read, `space e` focuses
  it, `ret` opens a file and gives focus back, `q` closes it.
- **`space v` and `space o` are menus,** like `space s`, so the key help panel lists the choices
  that apply (`]C` only in the per-commit scope, "since last review" only after a submit).

## Revisions in `glos diff`, and comments on any diff (2026-11-04)

- **Comments work on any diff,** two revisions and single commits included. First built
  read-only (comments only when the new side was your files); changed at the user's request: a
  reviewer comments on what they're looking at, and the agent is told what that was.
- **Each thread keeps its diff** (schema v4): the commits on each side and a sentence saying
  what was compared. It's what lets an agent tell a comment on an old commit from one on the
  current code.
- **Where a thread lives:** one made on your files is re-anchored as they change (as before);
  one made on a commit stays on that commit's code, which never changes. In any other view a
  thread is placed by its code, without storing anything, and marked with the version it was
  made on (`on d0c4165`, `on your files`); if its code isn't there it isn't shown inline.
- **Agents are told where the code is now:** with each comment, whether the same code is
  still in the files (and where), or has changed since. A thread on your files that simply
  followed its code needs no note.
- **Paths win:** an argument that exists on disk is a path; otherwise it's a revision.
- **The per-commit and two-revision diffs share the branch's review,** so their comments reach
  the agent with the rest.

## The key palette (2026-11-04)

- **`Ctrl-G` everywhere:** one key that shows the keys for the current context, insert mode
  included. `F1` needs `fn` on a Mac; `Ctrl-/` reaches the program differently per terminal;
  `Option` keys type characters on a Mac; `?` is Helix's reverse search and a typed character
  in text. `Ctrl-G` is unbound in Helix's every mode, picker and prompt.
- **It's Helix's command palette** (`space ?`), filtered to what applies, so it's one thing to
  learn: in the file view `ret` runs the command; elsewhere the keys are listed, and `esc` goes
  back to where you were (a covered picker comes back with its query).

## Dashboard, closing and launching (M5, 2026-11-04)

- **The dashboard is a picker** (columns: state, agent, unsent, unread, open, outdated, review),
  so typing filters and `C-g` lists its keys. `space l` opens it over a review; `glos dashboard`
  shows it on its own. Choosing another review closes this one and opens that one in the same
  terminal; `:q` still quits glos.
- **The agent's state** comes from the store: *waiting for review* (an open request),
  *replied* (unread replies), *submitted, not picked up* (no `wait_for_review` took the last
  round), *working* (it did). The agent column names the live MCP sessions serving the folder.
- **`:close` needs everything sent,** then quits. Running glos on the same source opens the
  review again, as there's one review per source.
- **Archiving on its own** happens when glos lists reviews (`glos list`, `glos dashboard`,
  `glos gc`): the folder is gone, the branch was deleted, or the branch was merged. A branch
  counts as merged when its tip is in the base, it had at least one round, and nothing is left
  to review there; a new branch with no commits looks merged otherwise.
- **`open-command` is a prefix:** glos adds its own command line (`<glos> --db <db> --review
  <id>`), since a new tmux or kitty pane doesn't inherit glos's folder or environment. So the
  setting is `open-command = "tmux split-window -h"`, not `... glos`.
- **A running glos registers itself** (schema v5, `viewers`), so `request_review` and `glos open`
  don't open a second one.
- **The man page is `glos man`** (hidden), not a build-time file: one place to keep it up to
  date, and packagers run `glos man > glos.1`.

## Helix in its own crate (2026-11-04)

- **`crates/glos-helix` (MPL-2.0) holds everything from Helix:** the git dependencies on
  `helix-core` and `helix-stdx`, `tree-house`, the copied `languages.toml`, queries and themes,
  the vendored grammars and the `build.rs` that compiles them, the commands ported from
  `helix-term`, and the highlighting code built on them.
- **glos never names Helix's crates.** Its `Cargo.toml` doesn't depend on them; it imports from
  glos-helix, which re-exports only what glos uses (`text`, `layout`, `test`), so that list is
  what to check on a Helix upgrade.
- **The highlighter takes the theme's scope names and a function to build styles,** so it needs
  neither glos's `Theme` nor ratatui. The ported `zm` takes glos's column function the same way.
- **Not a legal requirement:** MPL-2.0 is per file, so mixing was already allowed. The crate makes
  the boundary visible and leaves glos free to choose its own licence.

## Maintenance tasks in Rust (2026-11-04)

- **`cargo xtask`, not Python scripts:** `fetch-helix` and `keymap` are a small `xtask` crate in
  the workspace, so the project is Rust only. `ureq`, `flate2` and `tar` (downloading and
  unpacking) are its own dependencies, not glos's.
- **Checked against the scripts they replace:** `fetch-helix` rewrote `crates/glos-helix/` with
  no difference, and `keymap` produced the same checklist. The checklist lost its "Planned" table:
  every key in it is built.

## The plan folder

- **Chosen:** PLAN.md, this decision log, and a git repository. No Cargo skeleton yet.
- **Moved 2026-11-04** into `plans/`, with `KEYMAP.md`, the home for agents' plans from now on.
