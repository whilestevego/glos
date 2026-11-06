# One keymap for every context

glos binds keys in two different ways. The editor's keys go through `src/ui/keymap.rs`, which
is a trie of named commands with descriptions. Every other place that takes keys matches raw keys
in its own `key` method instead. Those places are the picker, the command line, the file tree and
the comment box. This plan moves all of them into the one keymap, so that every key glos knows
is a named command bound in one place.

This builds on [TREE.md](TREE.md), whose new keys it moves first. Do it after the tree PR is
merged.

Decided:

- One keymap holds every key. Each place that takes keys is a context with its own root menu.
- A component gets a command, not a key. The only raw input it still gets is text typed into a
  line or a box.
- The palette, the key help panel and `KEYMAP.md` are all built from the keymap. Nothing lists
  keys by hand.
- No key changes. Every key does what it does today.

## Where we are

`Keymaps` has two root menus, `normal` and `select`. `App::handle_key` walks one of them with
`pending` and the new key. When it reaches a command, it runs it through `App::execute`. Menus
open the key help panel, counts come before a command, and `Z` is sticky. The commands are in
`COMMANDS`, each with Helix's name and description and an `Action`.

Four components take keys before that walk, in this order. Each one decides with a `match` on
`KeyCode`:

| Component | Where | Keys |
|---|---|---|
| Picker | `Picker::key_paged` | Moving, paging, `ret`, `esc`, typing into the query |
| Command line | `Prompt::key` | `ret`, `esc`, `tab` completion, `backspace`, typing |
| File tree | `Tree::key` | `j`/`k`, `h`/`l`, `ret`, `/`, `?`, `n`, `N`, `esc`, `q` |
| Comment box | `CommentBox::key` | In insert mode, every key. In normal mode, `ret`, `q`, `i` and `a`, with the rest going on to the editor's keymap. |

`Ctrl-G` is checked before all of them, in `handle_key`.

This has three costs:

- The palette can't read these keys from anywhere, so `App::context_keys` lists them by hand.
  `picker::KEYS` does the same for the picker. Nothing checks either list against the `match`
  it describes. The tree PR had to change both by hand.
- `cargo xtask keymap` reads only `keymap.rs`, so `KEYMAP.md` can't list these keys.
- Remappable keys are planned for after the MVP (`PLAN.md`). As things stand, remapping could
  only reach the editor.

The split came from Helix. Helix's picker, prompt and file explorer also match raw keys in their
own `handle_event`, and their keys can't be remapped. `COMPONENTS.md` kept that split when it
turned these parts into components. This plan reverses that one part of `COMPONENTS.md`. The
components themselves stay as they are.

## Design

### Contexts

`Keymaps` gets one root menu per context, looked up by a `Context` enum:

| Context | When it's used |
|---|---|
| `Normal`, `Select` | The file view, as today |
| `Picker` | A picker or the palette is open |
| `Prompt` | The command line is open |
| `Tree` | The file tree has focus |
| `BoxInsert` | The comment box is in insert mode |
| `BoxNormal` | The comment box is in normal mode |

`App::context()` picks the context from what's on top. The order is the same as today's routing:
picker, command line, focused tree, comment box, and then the mode. One function now makes the
choice that `handle_key` and `context_keys` each make separately today.

A context can fall back to another one. `BoxNormal` holds only `ret`, `q`, `i` and `a`, and falls
back to `Normal`. A key that `BoxNormal` doesn't bind is looked up in `Normal`, as it reaches the
editor's keymap today. The commands that wait for the box to close (`blocked_in_box`) keep being
refused after lookup, as now. `Select` stays as it is, built from `normal_defs` with its two
differences.

### Commands

Each component's keys become named commands in `COMMANDS`, with descriptions. These become the
palette's text. Helix has no names for these, so glos names them with the context as a prefix:

- `tree_next`, `tree_prev`, `tree_open`, `tree_close_or_up`, `tree_open_or_into`,
  `tree_search`, `tree_rsearch`, `tree_search_next`, `tree_search_prev`, `tree_unfocus`,
  `tree_close`
- `picker_next`, `picker_prev`, `picker_page_down`, `picker_page_up`, `picker_first`,
  `picker_last`, `picker_choose`, `picker_close`, `picker_delete_char`
- `prompt_submit`, `prompt_cancel`, `prompt_complete_next`, `prompt_complete_prev`,
  `prompt_delete_char`
- `box_save`, `box_close`, `box_insert`, `box_append`, `box_normal_mode`, `box_newline`,
  `box_delete_char_backward`, `box_delete_char_forward`, and the insert-mode moves
  (`box_move_left` and the rest)
- `keys_palette`, "Show the keys for where you are", which is bound to `C-g` in every context

`Action` gets one variant per component, such as `Action::Tree(TreeCmd)`, where `TreeCmd` is that
component's enum of commands. A test checks that every context binds only commands meant for it,
plus the editor's commands for a context that falls back to `Normal`.

### Components get commands

The `Component` trait loses `key` and gains two methods:

```rust
trait Component {
    type Cmd;
    fn command(&mut self, cmd: Self::Cmd, cx: &Cx) -> Outcome;
    /// A character typed where text goes. Components without text input ignore it.
    fn text(&mut self, _c: char, _cx: &Cx) -> Outcome { Outcome::Ignored }
    // mouse and render are unchanged.
}
```

Each component's `match` on keys becomes a `match` on its own `Cmd`. The bodies stay the same.

Text input works as in Helix's insert mode. In a context with text input (`Picker`, `Prompt` and
`BoxInsert`), a key that isn't bound and is a plain character, with no `C-` or `A-`, goes to
`text`. In the other contexts, an unbound key is dropped, as the tree and the editor drop them
today.

### Routing

`handle_key` keeps one path:

1. `next_key` (after `f`, `t`, `mi`…) as today, in the editor contexts only.
2. A count, in `Normal`, `Select` and `BoxNormal` only, and only when nothing is pending.
3. Walk the context's menu with `pending` and the key, and fall back to the next context when
   the walk finds nothing at the root. Menus and sticky menus work in every context, though only
   the editor uses them for now.
4. A command runs. An editor command runs through `execute` as now. A component command goes to
   that component's `command`, and its `Outcome` goes through `apply` as now.
5. An unbound plain character in a text context goes to `text`.

`Ctrl-G` stops being a special case and becomes `keys_palette`, bound in every context. In the
`Picker` context, when the open picker is the palette, it closes the palette. That's what the
special case does now.

### What reads the keymap

- `context_keys` walks the context's menu with `walk_keys`, and then its fallback. The
  hand-written lists in `context_keys` and `picker::KEYS` are deleted. A context with text input
  adds one row, "type", whose description comes from the context. `applies` keeps rows out
  where they don't work, as it does for the editor. For example, `prompt_complete_next` applies
  only on the `:` line.
- The palette runs any command it lists, including component commands, which it can't do today.
  So choosing "Close the tree" in the palette closes the tree.
- `cargo xtask keymap` adds a table for each context after "Keys glos adds", such as "Keys in the
  file tree". Only normal mode is compared with Helix, as now.

## Steps

One PR on a `keymap-contexts` branch, one commit per step. Each step passes `cargo fmt`, `clippy`
and `cargo test` without changing any existing app test, apart from palette rows in the last step.

1. Add `Context`, root menus by context, fallback, and `App::context()`. Move `Normal` and
   `Select` onto them. Nothing changes for the user.
2. Move the tree onto the keymap: `TreeCmd`, `tree_defs()`, and `Tree::command`.
3. Move the picker, with text input.
4. Move the command line, with text input.
5. Move the comment box: `BoxInsert` with text input, and `BoxNormal` falling back to `Normal`.
6. Bind `C-g` as `keys_palette` everywhere and remove the special case. Build `context_keys` from
   the keymap and delete the hand-written lists. Extend `cargo xtask keymap`. Add an entry to
   `DECISIONS.md` that replaces the `COMPONENTS.md` decision this reverses. Write an "As built"
   section here.

## Tests

The main proof is that the existing app tests pass unchanged. They drive every component by its
keys, so they show that each key still does what it did.

New unit tests in `keymap.rs`:

- Each context resolves its keys to the expected commands.
- No key is bound twice in any menu of any context. This extends `no_key_is_bound_twice_in_a_menu`.
- Every command is bound in some context and has a description. This extends
  `every_command_is_bound_and_described`.
- Every context binds only its own commands, or the editor's through its fallback.
- `BoxNormal` binds `ret`, `q`, `i` and `a` itself, and reaches `Normal` for other keys such as
  `w`.
- Every context binds `C-g`.

New tests in `tests.rs`:

- In each context, the palette lists every command bound there that applies, with its keys. The
  test compares the palette with the keymap, not with a hand-written list.
- An unbound character is typed into the picker's query, the command line and the box, and is
  dropped in the tree.
- Choosing a component command in the palette runs it. "Close the tree" closes the tree, and
  "Next row" moves the tree's cursor.
- `C-g` opens and closes the palette from every context, as today.

New test in `xtask`:

- `KEYMAP.md` has a table for each context.

## Out of scope

- Remapping keys in `config.toml`. This plan makes it possible for every context, but adding it
  is a separate piece of work.
- Changing any key.
- The mouse. Clicks and the wheel stay in each component's `mouse`.
- Typable commands. The `:` commands stay as they are.

## As built

The work follows the plan, with the differences below.

There is no separate `keys_palette` command. `C-g` is bound to Helix's `command_palette` in every
context, and `space ?` stays bound to it in normal mode. Its description is now "Show the keys
for where you are". When it runs while the palette is open, it closes the palette, as the special
case used to. Choosing its row in the palette just goes back.

`C-g` still works partway through a sequence, as it did before. Before walking the keymap,
`App::handle_key` looks the key up at the context's root. If it runs `command_palette`, the
palette opens and the pending keys are dropped. The keymap decides which key that is, so the
rule doesn't name `C-g`.

`Component::key` went away in step 5, once nothing called it. Step 6 then added `command` and
`text` to the trait.

The box's normal-mode rows in the palette lost the `:editor` hint. That row described a typed
command and not a key, so the keymap has nothing to build it from.

Picker and comment box commands that used to share a row now have one row each. For example,
`i` and `a` are now "Insert before the selection" and "Insert after the selection". The
palette's normal-mode list is one row shorter, because `C-g` shares the row of `space ?`.
