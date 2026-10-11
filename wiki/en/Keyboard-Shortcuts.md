# Keyboard shortcuts

[简体中文](../zh-CN/Keyboard-Shortcuts.md) · [Home / 首页](Home.md)

The primary modifier is **Ctrl** on Windows/Linux and **Command (⌘)** on macOS. Saved user bindings override defaults. Open **Tools → Settings → Shortcuts** to record, clear or reset the ten configurable commands; duplicate nonempty bindings prevent saving.

| Configurable command | Windows / Linux | macOS |
| --- | --- | --- |
| Open file | Ctrl+O | ⌘O |
| Focus search | Ctrl+F | ⌘F |
| Quick find in focused region | Ctrl+Shift+F | ⌘⇧F |
| Close tab | Ctrl+W | ⌘W |
| Toggle settings window | Ctrl+, | ⌘, |
| Toggle case sensitivity | Alt+C | ⌥C |
| End of file | Ctrl+End | ⌘End |
| Cycle color label | Ctrl+D | ⌘D |
| Toggle wrapping | W | W |
| Add text tag to one line | N | N |

| Fixed command | Binding (primary = Ctrl / ⌘) |
| --- | --- |
| New native window | Primary+Shift+N |
| Full reload | F5 |
| Go to source line | Primary+G |
| Copy selected text / rows | Primary+C |
| Copy rows with source numbers | Primary+Shift+C |
| Select all rows in focused region | Primary+A |
| Mark / unmark rows | M |
| Start of file | Primary+Home |
| Enter edit mode | I, outside an input |
| Save editor or settings | Primary+S, in that window/context |
| Clipboard text as temporary file | Primary+V, outside an input; macOS also accepts Ctrl+V |
| Fullscreen | F11; macOS also Ctrl+⌘F |

Arrow keys, PageUp/PageDown and Home/End navigate a log region; Shift extends selection. Enter submits the main query. Quick find uses Enter/Shift+Enter or F3/Shift+F3 for next/previous and Esc to close. Tab/Shift+Tab moves focus. Ctrl+wheel adjusts log font size.

Search-tab navigation: Left/Right switches tabs, Home/End selects first/last, Alt+Left/Right reorders, F2 renames, Delete/Backspace closes, Enter returns to search input. Most workspace bindings exclude input controls so typing M/N/W/I and normal input copy/paste keep their text meaning. Settings use Primary+W or Esc to close and revert unsaved preview.

Source: [actions.rs](https://github.com/zhaiyanqi/vclogg2/blob/main/crates/vclogg-app/src/actions.rs).
