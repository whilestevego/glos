# glos

glos is a code reviewer for your terminal. It is made to review the work of coding agents such
as Claude Code.

When an agent finishes a task, you usually read what it changed and tell it what to fix. On
GitHub, you must push a branch, open a pull request and go to a browser. You must do this for
code that you possibly do not want to publish yet. With glos, you stay in your terminal. You
read the changes, comment on the lines you care about and send the comments to the agent. The
agent replies on the same lines, makes its fixes and asks you to look again. The review stays
open until you approve it.

If you use [Helix](https://helix-editor.com), glos is familiar to you. It has the keys, the
modes, the themes and the syntax highlighting of Helix.

## What it can review

- **A git branch.** Run `glos` in a repository. It shows all the changes of your branch since it
  left `main`, with uncommitted and untracked files.
- **Git revisions.** `glos diff HEAD~1` compares a commit with your files as they are now.
  `glos diff HEAD~3 HEAD~1` compares two commits.
- **Two files or two folders.** `glos diff old.rs new.rs` or `glos diff old/ new/`. This does
  not need git.

## Install

glos is written in Rust. You need [Rust](https://rustup.rs) and a C compiler, because glos
compiles the grammars for syntax highlighting into its binary. Then run:

```sh
cargo install --git https://github.com/whilestevego/glos
```

glos works on macOS and Linux. Then run `glos doctor` to check your setup.

## Connect your agent

glos talks to agents through MCP. To connect Claude Code, run this command once:

```sh
claude mcp add glos -- glos mcp
```

Then your agent can ask you for a review when it is done. It can wait for your comments, reply
to them and mark threads as resolved. glos never guesses what the agent wants to have reviewed.
The agent names the branch and its base, or the two files or folders. It then gets a review id,
which it uses for each later call. Before you start, the agent can leave its own notes on its
changes, such as "I was not sure about this part". It can also point you at a line.

Agents without MCP can do the same things from the command line. `glos open-review git . topic
main` (or `glos open-review diff old new`) prints the id of a review. `glos reply`,
`glos resolve`, `glos annotate`, `glos show` and the other commands take it as `--review <id>`.
Add `--json` to a command to get output that a program can read.

## A review from start to end

1. The agent finishes its work and asks for a review. If glos is open, the request shows at the
   bottom of the screen. If not, run `glos` in the project folder.
2. Read the changes. All changed files are in one scroll, each under its own header. glos folds
   unchanged code, and leaves three lines around each change. Each removed line shows in red
   above the lines that replaced it, and added lines show in green. `]g` goes to the next change
   and `gn` goes to the next file. The file tree on the left shows each changed file.
3. To comment, put the cursor on a line (or select lines with `x`) and press `' c`. Type your
   comment, press `esc`, then press `enter` to save it. glos saves each comment as you type, so
   you do not lose it if you quit or glos crashes. A comment on removed lines stays with those
   lines. The agent sees it as `path:-12`, with a minus sign before the line number.
4. When you are done, press `' s` and pick a verdict: approve, request changes, or only send
   the comments. The agent gets all your comments at the same time.
5. While the agent works, glos updates itself and does not move your screen. The replies of the
   agent show under your comments, marked unread. Press `]u` to go to the next one.
6. For the next round, press `' o` then `r` to see only the changes since your last review.
   Reply, resolve or reopen threads, and submit again.
7. After you approve, `:close` closes the review.

If your code changes under a comment, glos moves the comment with the code. If the code of the
comment is gone, glos marks the comment outdated and shows the code as it was. A comment on
removed lines is outdated when the agent puts those lines back.

## Keys

glos uses the keys of Helix to move and to select: `hjkl`, `w`, `b`, `gg`, `x`, `v`, `/` and the
others. It adds these keys to review:

| Key | What it does |
|---|---|
| `' c` | Comment on the selected lines |
| `' C` | Comment on the review as a whole |
| `' r` | Reply to the comment under the cursor |
| `' m` | Edit your latest comment |
| `' M` | Delete your latest comment, if it is not sent yet |
| `' x` | Resolve or reopen a thread |
| `' s` | Submit the review |
| `' v` | Change the view: unified or side by side, or one file at a time |
| `' o` | Change what is shown: the whole branch, the changes since your last review, one commit at a time, or a filter |
| `' d` | Pick a thread in this file (`' D` for the whole review) |
| `' l` | The dashboard: every open review |
| `' n` | Go to the line that the agent asked you to look at |
| `space g` | Pick a changed file |
| `space f` | Open any file in the project, for context |
| `space e` | Open, focus or close the file tree |
| `]r` `[r` | Next or previous thread (`]R` `[R` for the last and first) |
| `]u` `[u` | Next or previous unread reply |
| `gn` `gp` | Next or previous changed file |
| `enter` | Open or collapse the thread under the cursor, or open a fold |

The review commands are under `'`, and the thread commands use `]r` and `[r`, so that Helix
keeps its own keys. `space c`, `space C`, `space r`, `space s`, `space d`, `space D` and `]d`
are free for the editing and language-server commands of Helix.

### All files in one scroll

glos shows all changed files in one scroll, as the "Files changed" page of GitHub does. Each
file starts with a rule across the screen. The rule names the path, the added and removed lines
and the open threads of the file. While you scroll through a file, its header stays at the top
of the screen. `' v a` changes to one file at a time and back, and so do `:one` and `:all`. To
start glos with one file at a time, set `all-files = false` in the settings.

Click the header of a file to collapse the file, and click it again to expand it. A collapsed
file shows a line such as `↕ 42 lines` under its header in place of its code. A click on that
line also expands the file. `enter` does the same for the file under the cursor, unless the
cursor is on a fold or a thread. A collapsed file keeps its folds, so it opens again as you left
it.

In one scroll, `j` and `k` go on into the next and previous file. `gg` goes to the start of the
first file, and `G` goes to the end of the last file. The page keys scroll through the whole
review. `gn`, `gp`, the tree and `space g` go to the header of a file. glos reads the other
files only when they come near the screen, so a big review opens as fast as one file. This works
in each view, so side by side shows the old and new sides of each file under its header.

The mouse wheel scrolls through all the files, and a click in the code of a file moves the
cursor there. Search, `]g`, `]p`, `]f` and the other jumps go on into the next file when this
file has no more matches. `n` goes from the last file to the first. `]G` and `[G` go to the last
and first change of the review, as `]R` and `[R` go to its last and first thread. These also
work on the whole review: a count such as `5j`, `z j` and `z k`, `zz`, `zt`, `zb`, `gt`, `gc`
and `gb`.

### Folding

glos folds lines far from a change into a band across the screen. The band says which lines it
hides, such as `↕ lines 16–31 (16)  ret opens`. glos never folds a line that has a comment.
Press `enter` on a fold, or click it, to open it. The other folding keys are under `z`, as in
Vim:

| Key | What it does |
|---|---|
| `zo` | Open the fold under the cursor |
| `za` | Open or close the fold under the cursor |
| `zR` | Open every fold in the file |
| `zM` | Fold everything again |
| `z+` `z-` | Show one more or one fewer line around each change |
| `zf` | Fold the selected lines yourself |
| `zd` | Remove a fold that you made |

A fold that you make can hide anything, also changes and comments. It says what it hides:
`↕ lines 40–51 (12) · 3 changed · 1 thread`. A jump to something in it (`]g`, `]r`, a picker)
opens it again.

To fold a whole function, select it with `maf` and press `zf`. glos has the syntax-aware
selections of Helix. `maf` and `mif` select a function, `mat` a type, `mac` a comment and `maa`
an argument. `]f`, `[f`, `]t` and the related keys go from one to the next.

You do not need to remember these keys. Press `space` (or `g`, `]` and the others) and wait. A
panel then lists the keys that can come next. Press **`Ctrl-G` anywhere** to see each key that
works where you are: in the file view, the comment box, the file tree or a list. The top
border of the panel names the context of those keys, such as `normal`, `tree` or `box_insert`.

The mouse also works. Click to move the cursor, drag to select, and click a thread to open it.
A click on the file tree or the code moves the keyboard focus there. In the tree, `ret` or a
click opens and closes a folder. `h` and `l` close and open folders, as in the file explorer of
Helix, and `/` searches the names. The scroll wheel scrolls the panel under the pointer. This
includes the tree, the comment box, the key menu and the preview of the picker. When lines are
not wrapped, you can scroll the code to the side. Swipe on a touchpad, or hold Shift while you
turn the wheel.

## Several agents at the same time

Agents can work on several branches or worktrees at the same time. `glos dashboard` (or `' l`
in glos) then lists each open review. Each row shows what the agent does and what waits for
you, such as unsent comments and unread replies. Press `enter` to open a review.

## Settings

The settings go in `~/.config/glos/config.toml`:

```toml
# Any Helix theme. glos comes with all of them, and uses catppuccin_mocha unless you
# choose another. "default" is Helix's own default theme.
theme = "catppuccin_mocha"

# Optional: how glos opens itself in a new pane when an agent asks for a review.
# Give the start of the command; glos adds the rest.
open-command = "tmux split-window -h"

[editor]
mouse = true          # set to false to turn the mouse off
file-tree = true      # show the file tree when glos starts
diff-context = 3      # unchanged lines shown around each change; the rest are folded
all-files = true      # every changed file in one scroll; false shows one file at a time

[editor.soft-wrap]
enable = false        # wrap long lines
```

For kitty, use `open-command = "kitten @ launch --location=vsplit"`. For WezTerm, use
`open-command = "wezterm cli split-pane --"`. Without an `open-command`, glos never opens
windows itself, and the agent asks you to run `glos`.

You can put your own themes in `~/.config/glos/themes/`, in the format of Helix. Folds use
`ui.virtual.fold` if your theme sets it. If not, they use the colour of the current line.

Changed lines get a tinted background, and the changed words in them get a stronger tint. Helix
themes have no keys for these tints, so glos calculates them from your theme. It mixes
`ui.background` toward `diff.plus` or `diff.minus`. A theme can set its own tints with four keys
that Helix ignores:

| Key | Used for |
|---|---|
| `diff.plus.line` | Added and modified lines |
| `diff.minus.line` | Removed lines |
| `diff.plus.word` | Changed words in an added line |
| `diff.minus.word` | Changed words in a removed line |

A theme without `ui.background` uses the background of the terminal, which glos cannot know. In
such a theme, only the gutter marks changed lines, unless the theme sets these keys.

## Other commands

| Command | What it does |
|---|---|
| `glos list` | List open reviews (`--all` includes closed ones) |
| `glos show --review <id>` | Print the comments and replies of a review |
| `glos open --review <id>` | Open glos on a review, with your `open-command` |
| `glos close --review <id>` | Close a review |
| `glos gc` | Remove old, finished reviews |
| `glos completions zsh` | Shell completions (also `bash`, `fish` and others) |
| `glos doctor` | Check your setup |

## Where your data is

All your data stays on your machine. glos keeps the reviews in one database at
`~/.local/share/glos/glos.db`, and writes its logs to `~/.local/state/glos/glos.log`. glos
sends data only to the agents that you connect.

A review closes itself when its branch is merged or deleted. `glos gc` removes reviews that
were finished more than 30 days ago.

## Development

Run `cargo xtask setup` once to install the git hooks. Run `cargo xtask ci` before each commit.
It runs the same checks as CI. `CLAUDE.md` gives the rules and the commands.

## Credits

glos takes a lot from Helix: its editing core, its themes, its syntax queries and its key
layout. All code from Helix is in its own crate, `crates/glos-helix`, which keeps the licence of
Helix, MPL-2.0.
