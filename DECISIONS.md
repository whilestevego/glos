# Decisions

Each decision made while planning glos on 2026-11-03: the options considered, what was chosen, and
why. The design itself is in [PLAN.md](PLAN.md).

## Why build it at all

- **Considered:** a Claude Code pane mod; Plannotator, Preflight or Diffsmith (browser or desktop
  apps); tuicr in a Ghostty split, which was tried and wired up to Claude Code.
- **Chosen:** a dedicated terminal tool.
- **Why:** the pane mod is capped at 45% of the terminal width in fullscreen and can't fit long
  lines. Browser and desktop tools leave the terminal. tuicr works, but has no hooks for agents,
  loses comments unless you save before quitting, and its interface isn't Helix.

## Agent integration

- **Options:** MCP server; event hooks; CLI + JSON; local socket/RPC.
- **Chosen:** MCP server, CLI + JSON, and a local socket.
- **Why:** MCP lets any agent use glos natively; the CLI serves scripts and agents without MCP; the
  socket lets replies appear live in an open UI.

## Delivering a submitted review

- **Options:** MCP channel push; a Claude Code mod that submits a prompt; the agent polls
  (`wait_for_review`).
- **Chosen:** MCP channel push.
- **Why:** no mod is needed, and the review arrives as you submit. The other two remain as fallbacks
  (`wait_for_review` is in the tool list) pending the M0 spike.

## Helix fidelity

- **Options:** keymap and modes; pickers and popups; themes and config; tree-sitter.
- **Chosen:** all four.

## Foundation

- **Options:** ratatui with Helix-style behaviour; Helix's crates as git dependencies; a fork of
  Helix.
- **Chosen:** ratatui with Helix-style behaviour.
- **Why:** stable crates.io dependencies. Helix's crates have no stable API and pull in the whole
  editor, and a fork means maintaining a large editor.

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

## Diff views

- **Options:** unified with a split toggle; side by side with a toggle; full file with a gutter.
- **Chosen:** all three, full file + gutter by default, and switching must be easy (`space v`).

## Threads

- **Options:** resolve and reopen; outdated tracking; comment types; review-level comments.
- **Chosen:** resolve and reopen, outdated tracking, review-level comments.
- **Not chosen:** comment types (issue, nit, praise…).

## While the agent works

- **Options:** live refresh; refresh on demand; snapshot rounds.
- **Chosen:** live refresh. Rounds still exist in the data model, to support the
  "since last review" scope.

## Git library

- **Options:** gix; git2; shelling out to git.
- **Chosen:** gix. It's pure Rust and what Helix uses for its diff gutter.

## Review keys

- **Options:** space-mode menu; bracket motions; typable commands; selection-first comments.
- **Chosen:** all four.

## What agents can do

- **Options:** reply and resolve; pre-annotate their changes; request review; point you to a line.
- **Chosen:** all four. Pointing you to a line works only with your permission, per review.

## Launching

- **Options:** a standalone command; from Claude Code; when the agent asks.
- **Chosen:** all three.

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
- **Chosen:** reuse Helix's runtime. Exactly your editor's colours; without Helix, plain text.

## MVP

- **Options:** reading and commenting; threads both ways; all three views; the dashboard.
- **Chosen:** all four. In-place editing follows the MVP.

## Name

- **First pick:** loupe (a jeweller's magnifier), dropped because it clashes with GNOME's image
  viewer (its command is `loupe` on many Linux desktops) and an existing crate.
- **Then:** about 100 names were checked against crates.io, Homebrew, Linux distro packages
  (Repology), commands on the PATH and GitHub. Long names (kibitzer, obelus, margent, scholion)
  were rejected as too long; most short words (stet, gloss, lgtm, ptal, nits, quirk) were taken.
- **Chosen:** glos, from *gloss*: a note written in the margin of a text. Four letters; free on
  crates.io, Homebrew and Linux distros, with no notable GitHub project of that name (checked
  2026-11-03). Trademarks and domains were not checked.

## The plan folder

- **Chosen:** PLAN.md, this decision log, and a git repository. No Cargo skeleton yet.
