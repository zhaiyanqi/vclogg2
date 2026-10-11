# Settings

[简体中文](../zh-CN/Settings.md) · [Home / 首页](Home.md)

Open **Tools → Settings** or **Primary+,**. The process shares one native settings window; menu entries can reveal it at a category. It is resizable through the system frame and remembers size. Changes use drafts and cross-window preview. **Primary+S** saves; cancel, Esc or close reverts uncommitted preview. Immediate history deletion is a separate operation and is not rolled back.

| Category | Contents |
| --- | --- |
| General | Language, paths, close/paste confirmation, directory command, wheel and word boundaries |
| Network | Optional cloud server connection and session configuration |
| Appearance | Theme, log font/size/spacing, line numbers, search control sizing |
| Highlight | Coloring groups, labels, selection and keyword-match styles |
| Search | Case, regex, per-file result limit, match highlighting |
| History | File records, temporary results, search history |
| Storage | Database/index statistics, reveal directories and cleanup |
| Shortcuts | Ten configurable commands with collision checks |
| Advanced | Built-in icons, application log level and export |
| About | Version, locally displayed build time and project links |

New configurations default to Chinese, the light theme, Consolas 13 px with 6 px extra line spacing, a 60 px line-number column, and unlimited results (0). Case sensitivity, regex and log-level coloring start off; committed-match highlighting starts on. Debug builds default to Debug logging and Release to Error. Existing saved settings and per-file session state can override defaults.

Toolbar controls and the main search input share a 36 px default height, with independently configurable 13 px default font sizes. The actual common height expands to fit the fonts. Fonts absent on another machine retain their saved name but use available rendering fallbacks.

View → Horizontal scrollbar synchronizes visibility across windows; horizontal wheel navigation remains available when it is hidden. Per-file line numbers, separators and wrapping are distinct from defaults for new files. Themes support system/light/dark. macOS system-window menu language is initialized at startup, so save and restart to change those native resources.

The View menu notification switch suppresses new popups, while the current-run history retains up to 100 shared messages. Four built-in icon choices affect supported runtime surfaces without rewriting signed packages or guaranteeing every pinned OS shortcut changes.
