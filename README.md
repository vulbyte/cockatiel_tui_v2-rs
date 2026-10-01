# Cockatiel TUI v2

A Blender-inspired Binary Space Partitioning (BSP) layout engine for the
Cockatiel chat engine. This is the **new** TUI, built as its own module.

## What makes it different from the v1 TUI

- **BSP layout tree** — the screen is a root rectangle recursively split into
  panes (H/V + ratio). Panes can be split, joined, resized, and re-arranged at
  runtime — not the fixed 5-pane grid of the v1 TUI.
- **Per-pane view dropdown** — every pane header shows `[v] <view_type>`.
  Click it (or press `Ctrl+T`) to swap that pane's content to any view:
  `cockatiel_info`, `logs`, `module_manager`, `engine_graph`, `prompts`, or
  `top_users`.
- **`top_users` is a first-class mountable pane** — no longer pop-out-only.
- **Persistence** — the layout tree (splits, ratios, view assignments) is
  saved to `layout.json` on quit / after every structural change, and restored
  on launch.
- **Accessibility** — spatial TTS announcements on focus moves (macOS `say`,
  Linux `spd-say`, Windows SAPI), an F1 help modal, and a guaranteed ESC
  safe-route.
- **Keymap config** — `hotkey_config.json` accepts the spec's
  `{global_context, window_management}` schema (legacy `{nav, modules, chart,
  editor}` still loads).

## Keybinds (defaults)

| Action | Key |
|---|---|
| Focus next / prev | `Tab` / `Shift+Tab` |
| Split pane vertically | `Ctrl+v` |
| Split pane horizontally | `Ctrl+h` |
| Join pane into sibling | `Ctrl+w` |
| Change pane view | `Ctrl+T` |
| Accessibility help | `F1` |
| Toggle TTS announcements | `Ctrl+u` |
| Quit | `q` (double-`Esc` also works) |

## Layout persistence

The tree is stored as `layout.json` next to the TUI's `config.json`. It records
only the STRUCTURE (splits/ratios/leaf views), not window state — windows are
re-created fresh on load. Delete `layout.json` to reset to the default 5-pane
arrangement.

## Modules

This is a full TUI module: it supervises the engine + user database + all
modules, exactly like the v1 TUI (same supervisor/WS machinery), but renders
them through the BSP pane model instead of a fixed grid.

## Building

```sh
cargo build --release
```

Launch with `./target/release/cockatiel-tui-v2` (or `--no-engine` to skip
launching the engine). Run the test suite with `cargo test`.