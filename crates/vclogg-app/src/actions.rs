use gpui::{Action, App, KeyBinding, Keystroke, Unbind, actions};

use crate::state_store::ShortcutSettings;

pub const WORKSPACE_CONTEXT: &str = "VCLogg2Workspace";
pub const LOG_TABLE_CONTEXT: &str = "VCLogg2LogTable";
// Keep application commands from preempting text entry in gpui-component inputs.
const WORKSPACE_SHORTCUT_CONTEXT: &str = "VCLogg2Workspace && !Input";
const LOG_TABLE_SHORTCUT_CONTEXT: &str = "VCLogg2LogTable && !Input";

actions!(
    vclogg2,
    [
        OpenFiles,
        NewWindow,
        ReloadActive,
        CloseActiveTab,
        CopyCurrentLine,
        CopyCurrentLineWithNumber,
        SelectAllRows,
        ExtendSelectionUp,
        ExtendSelectionDown,
        ExtendSelectionPageUp,
        ExtendSelectionPageDown,
        ExtendSelectionFirst,
        ExtendSelectionLast,
        CopyFilePath,
        GoToLine,
        ToggleMarkedRow,
        CycleColorLabel,
        FocusSearch,
        OpenQuickFind,
        OpenSettings,
        StartSearch,
        CancelSearch,
        ClearSearch,
        ToggleCaseSensitive,
        ToggleRegex,
        ToggleWordWrap,
        OpenSearchResultsInNewTab,
        MergeSearchResultsInNewTab,
        SaveSearchResultsToFile,
        JumpToStart,
        JumpToEnd,
        ToggleFullscreen,
    ]
);

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("ctrl-o", OpenFiles, Some(WORKSPACE_SHORTCUT_CONTEXT)),
        KeyBinding::new("ctrl-shift-n", NewWindow, Some(WORKSPACE_SHORTCUT_CONTEXT)),
        KeyBinding::new("f5", ReloadActive, Some(WORKSPACE_SHORTCUT_CONTEXT)),
        KeyBinding::new("ctrl-w", CloseActiveTab, Some(WORKSPACE_SHORTCUT_CONTEXT)),
        KeyBinding::new(
            "ctrl-shift-c",
            CopyCurrentLineWithNumber,
            Some(LOG_TABLE_SHORTCUT_CONTEXT),
        ),
        KeyBinding::new("ctrl-c", CopyCurrentLine, Some(LOG_TABLE_SHORTCUT_CONTEXT)),
        KeyBinding::new("ctrl-a", SelectAllRows, Some(LOG_TABLE_SHORTCUT_CONTEXT)),
        KeyBinding::new(
            "shift-up",
            ExtendSelectionUp,
            Some(LOG_TABLE_SHORTCUT_CONTEXT),
        ),
        KeyBinding::new(
            "shift-down",
            ExtendSelectionDown,
            Some(LOG_TABLE_SHORTCUT_CONTEXT),
        ),
        KeyBinding::new(
            "shift-pageup",
            ExtendSelectionPageUp,
            Some(LOG_TABLE_SHORTCUT_CONTEXT),
        ),
        KeyBinding::new(
            "shift-pagedown",
            ExtendSelectionPageDown,
            Some(LOG_TABLE_SHORTCUT_CONTEXT),
        ),
        KeyBinding::new(
            "shift-home",
            ExtendSelectionFirst,
            Some(LOG_TABLE_SHORTCUT_CONTEXT),
        ),
        KeyBinding::new(
            "shift-end",
            ExtendSelectionLast,
            Some(LOG_TABLE_SHORTCUT_CONTEXT),
        ),
        KeyBinding::new("ctrl-g", GoToLine, Some(WORKSPACE_SHORTCUT_CONTEXT)),
        KeyBinding::new("m", ToggleMarkedRow, Some(LOG_TABLE_SHORTCUT_CONTEXT)),
        KeyBinding::new("ctrl-d", CycleColorLabel, Some(WORKSPACE_SHORTCUT_CONTEXT)),
        KeyBinding::new("ctrl-f", FocusSearch, Some(WORKSPACE_SHORTCUT_CONTEXT)),
        KeyBinding::new(
            "ctrl-shift-f",
            OpenQuickFind,
            Some(WORKSPACE_SHORTCUT_CONTEXT),
        ),
        KeyBinding::new("ctrl-,", OpenSettings, Some(WORKSPACE_SHORTCUT_CONTEXT)),
        KeyBinding::new(
            "alt-c",
            ToggleCaseSensitive,
            Some(WORKSPACE_SHORTCUT_CONTEXT),
        ),
        KeyBinding::new("w", ToggleWordWrap, Some(WORKSPACE_SHORTCUT_CONTEXT)),
        KeyBinding::new("ctrl-home", JumpToStart, Some(WORKSPACE_SHORTCUT_CONTEXT)),
        KeyBinding::new("ctrl-end", JumpToEnd, Some(WORKSPACE_SHORTCUT_CONTEXT)),
        KeyBinding::new("f11", ToggleFullscreen, Some(WORKSPACE_SHORTCUT_CONTEXT)),
    ]);
}

pub fn shortcut_to_key_binding(shortcut: &str) -> Option<String> {
    let parts = shortcut
        .trim()
        .split('+')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    let (key, modifiers) = parts.split_last()?;
    let mut normalized = Vec::with_capacity(parts.len());
    for modifier in modifiers {
        normalized.push(match modifier.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => "ctrl".to_string(),
            "alt" => "alt".to_string(),
            "shift" => "shift".to_string(),
            "meta" | "cmd" | "command" => "platform".to_string(),
            _ => return None,
        });
    }
    normalized.push(match key.to_ascii_lowercase().as_str() {
        "space" => "space".into(),
        value => value.to_string(),
    });
    let normalized = normalized.join("-");
    Keystroke::parse(&normalized).ok()?;
    Some(normalized)
}

pub fn apply_shortcuts(previous: &ShortcutSettings, next: &ShortcutSettings, cx: &mut App) {
    let mut bindings = Vec::new();
    rebind(
        &mut bindings,
        &previous.open_file,
        &next.open_file,
        OpenFiles,
        WORKSPACE_SHORTCUT_CONTEXT,
    );
    rebind(
        &mut bindings,
        &previous.focus_search,
        &next.focus_search,
        FocusSearch,
        WORKSPACE_SHORTCUT_CONTEXT,
    );
    rebind(
        &mut bindings,
        &previous.quick_find,
        &next.quick_find,
        OpenQuickFind,
        WORKSPACE_SHORTCUT_CONTEXT,
    );
    rebind(
        &mut bindings,
        &previous.close_tab,
        &next.close_tab,
        CloseActiveTab,
        WORKSPACE_SHORTCUT_CONTEXT,
    );
    rebind(
        &mut bindings,
        &previous.open_settings,
        &next.open_settings,
        OpenSettings,
        WORKSPACE_SHORTCUT_CONTEXT,
    );
    rebind(
        &mut bindings,
        &previous.toggle_case_sensitive,
        &next.toggle_case_sensitive,
        ToggleCaseSensitive,
        WORKSPACE_SHORTCUT_CONTEXT,
    );
    rebind(
        &mut bindings,
        &previous.jump_to_bottom,
        &next.jump_to_bottom,
        JumpToEnd,
        WORKSPACE_SHORTCUT_CONTEXT,
    );
    rebind(
        &mut bindings,
        &previous.cycle_color_label,
        &next.cycle_color_label,
        CycleColorLabel,
        WORKSPACE_SHORTCUT_CONTEXT,
    );
    rebind(
        &mut bindings,
        &previous.toggle_word_wrap,
        &next.toggle_word_wrap,
        ToggleWordWrap,
        WORKSPACE_SHORTCUT_CONTEXT,
    );
    cx.bind_keys(bindings);
}

fn rebind<A: Action + Clone>(
    bindings: &mut Vec<KeyBinding>,
    previous: &str,
    next: &str,
    action: A,
    context: &'static str,
) {
    if previous.eq_ignore_ascii_case(next) {
        return;
    }
    if let Some(previous) = shortcut_to_key_binding(previous) {
        bindings.push(KeyBinding::new(
            &previous,
            Unbind(action.name().into()),
            Some(context),
        ));
    }
    if let Some(next) = shortcut_to_key_binding(next) {
        bindings.push(KeyBinding::new(&next, action, Some(context)));
    }
}
