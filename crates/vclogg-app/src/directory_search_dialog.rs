use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use gpui::{
    AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    PathPromptOptions, Render, Styled as _, Subscription, Task, Window, div,
    prelude::FluentBuilder as _,
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, IconName, Sizable as _, StyledExt as _,
    button::Button,
    checkbox::Checkbox,
    h_flex,
    input::{Input, InputEvent, InputState},
    v_flex,
};

const DEFAULT_FILE_TYPE_PATTERNS: &str =
    "*.log|*.vclog|*.out|*.trace|*.txt|*.text|*.csv|*.json|*.xml|*.yaml|*.yml";
const LEGACY_LOG_FILE_TYPE_PATTERNS: &str = "*.log|*.vclog|*.out|*.trace";
const LEGACY_TEXT_FILE_TYPE_PATTERNS: &str = "*.txt|*.text|*.csv|*.json|*.xml|*.yaml|*.yml";

#[derive(Debug)]
struct DirectorySearchFileFilter {
    accepts_all: bool,
    suffixes: Vec<String>,
}

impl DirectorySearchFileFilter {
    fn new(enabled: bool, patterns: &str) -> Self {
        if !enabled {
            return Self {
                accepts_all: true,
                suffixes: Vec::new(),
            };
        }

        let mut accepts_all = false;
        let suffixes = patterns
            .split('|')
            .filter_map(|pattern| {
                let pattern = pattern.trim();
                if pattern.is_empty() {
                    return None;
                }
                if matches!(pattern, "*" | "*.*") {
                    accepts_all = true;
                    return None;
                }
                let suffix = pattern.strip_prefix('*').unwrap_or(pattern);
                let suffix = suffix.trim_start_matches('.');
                (!suffix.is_empty()).then(|| format!(".{suffix}").to_ascii_lowercase())
            })
            .collect();
        Self {
            accepts_all,
            suffixes,
        }
    }

    fn has_pattern(&self) -> bool {
        self.accepts_all || !self.suffixes.is_empty()
    }

    fn accepts(&self, path: &Path) -> bool {
        if self.accepts_all {
            return true;
        }
        let file_name = path
            .file_name()
            .map(|name| name.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        self.suffixes
            .iter()
            .any(|suffix| file_name.ends_with(suffix))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DirectorySearchOptions {
    pub directory: Option<PathBuf>,
    pub file_type_filter_enabled: bool,
    pub file_type_patterns: String,
    pub include_subdirectories: bool,
    pub include_hidden_directories: bool,
}

impl DirectorySearchOptions {
    pub(crate) fn from_legacy_file_type(file_type: u8) -> (bool, String) {
        match file_type {
            1 => (true, LEGACY_LOG_FILE_TYPE_PATTERNS.to_string()),
            2 => (true, LEGACY_TEXT_FILE_TYPE_PATTERNS.to_string()),
            3 => (false, DEFAULT_FILE_TYPE_PATTERNS.to_string()),
            _ => (true, DEFAULT_FILE_TYPE_PATTERNS.to_string()),
        }
    }
}

impl Default for DirectorySearchOptions {
    fn default() -> Self {
        Self {
            directory: None,
            file_type_filter_enabled: true,
            file_type_patterns: DEFAULT_FILE_TYPE_PATTERNS.to_string(),
            include_subdirectories: true,
            include_hidden_directories: false,
        }
    }
}

pub struct DirectorySearchEnumeration {
    pub paths: Vec<PathBuf>,
    pub unreadable_directory_count: usize,
}

pub fn enumerate_directory_search_paths(
    options: &DirectorySearchOptions,
) -> Result<DirectorySearchEnumeration> {
    let Some(root) = options.directory.as_deref() else {
        bail!(crate::tr!(
            "尚未选择搜索目录",
            "No search directory selected"
        ));
    };
    if !root.is_dir() {
        bail!(crate::tr_args!(
            "搜索目录不存在或不可访问：{}",
            "The search directory doesn’t exist or can’t be accessed: {}",
            root.display()
        ));
    }

    let file_filter = DirectorySearchFileFilter::new(
        options.file_type_filter_enabled,
        &options.file_type_patterns,
    );
    let mut paths = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    let mut unreadable_directory_count = 0;
    while let Some(directory) = pending.pop() {
        let entries = match std::fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if directory == root => {
                return Err(error)
                    .with_context(|| format!("无法读取搜索目录：{}", directory.display()));
            }
            Err(_) => {
                unreadable_directory_count += 1;
                continue;
            }
        };
        for entry in entries.flatten() {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if file_type.is_dir() {
                if options.include_subdirectories
                    && (options.include_hidden_directories || !is_hidden_directory(&entry))
                {
                    pending.push(path);
                }
            } else if file_type.is_file() && file_filter.accepts(&path) {
                paths.push(path);
            }
        }
    }
    paths.sort_by(|left, right| {
        left.to_string_lossy()
            .to_lowercase()
            .cmp(&right.to_string_lossy().to_lowercase())
    });
    Ok(DirectorySearchEnumeration {
        paths,
        unreadable_directory_count,
    })
}

fn is_hidden_directory(entry: &std::fs::DirEntry) -> bool {
    if entry.file_name().to_string_lossy().starts_with('.') {
        return true;
    }
    is_hidden_by_platform(entry)
}

#[cfg(windows)]
fn is_hidden_by_platform(entry: &std::fs::DirEntry) -> bool {
    use std::os::windows::fs::MetadataExt as _;

    const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
    entry
        .metadata()
        .is_ok_and(|metadata| metadata.file_attributes() & FILE_ATTRIBUTE_HIDDEN != 0)
}

#[cfg(not(windows))]
fn is_hidden_by_platform(_: &std::fs::DirEntry) -> bool {
    false
}

pub struct DirectorySearchDialog {
    directory: Option<PathBuf>,
    file_type_filter_enabled: bool,
    file_type_patterns: Entity<InputState>,
    include_subdirectories: bool,
    include_hidden_directories: bool,
    directory_validation_error: bool,
    file_type_validation_error: bool,
    directory_task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl DirectorySearchDialog {
    pub fn new(
        options: DirectorySearchOptions,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let file_type_patterns = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(options.file_type_patterns)
                .placeholder(crate::tr!("例如：*.txt|*.log", "Example: *.txt|*.log"))
        });
        let subscriptions =
            vec![
                cx.subscribe(&file_type_patterns, |this, _, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        this.file_type_validation_error = false;
                        cx.notify();
                    }
                }),
            ];
        Self {
            directory: options.directory,
            file_type_filter_enabled: options.file_type_filter_enabled,
            file_type_patterns,
            include_subdirectories: options.include_subdirectories,
            include_hidden_directories: options.include_hidden_directories,
            directory_validation_error: false,
            file_type_validation_error: false,
            directory_task: None,
            _subscriptions: subscriptions,
        }
    }

    pub fn options(&self, cx: &gpui::App) -> Option<DirectorySearchOptions> {
        let file_type_patterns = self.file_type_patterns.read(cx).value().trim().to_string();
        if self.file_type_filter_enabled
            && !DirectorySearchFileFilter::new(true, &file_type_patterns).has_pattern()
        {
            return None;
        }
        Some(DirectorySearchOptions {
            directory: Some(self.directory.clone()?),
            file_type_filter_enabled: self.file_type_filter_enabled,
            file_type_patterns,
            include_subdirectories: self.include_subdirectories,
            include_hidden_directories: self.include_hidden_directories,
        })
    }

    pub fn show_validation_errors(&mut self, cx: &mut Context<Self>) {
        self.directory_validation_error = self.directory.is_none();
        self.file_type_validation_error = self.file_type_filter_enabled
            && !DirectorySearchFileFilter::new(
                true,
                self.file_type_patterns.read(cx).value().as_ref(),
            )
            .has_pattern();
        cx.notify();
    }

    fn choose_directory(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.directory_task.is_some() {
            return;
        }
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(crate::tr!("选择搜索目录", "Select search directory").into()),
        });
        self.directory_task = Some(cx.spawn_in(window, async move |this, cx| {
            let directory = prompt
                .await
                .ok()
                .and_then(Result::ok)
                .flatten()
                .and_then(|mut paths| paths.pop());
            _ = this.update_in(cx, |this, _, cx| {
                this.directory_task = None;
                if let Some(directory) = directory {
                    this.directory = Some(directory);
                    this.directory_validation_error = false;
                }
                cx.notify();
            });
        }));
    }
}

impl Render for DirectorySearchDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _performance_scope = crate::ui_performance::scope("DirectorySearchDialog::render");
        let choosing = self.directory_task.is_some();
        let path = self
            .directory
            .as_deref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| crate::tr!("尚未选择目录", "No directory selected").to_string());

        v_flex()
            .id("directory-search-dialog")
            .w_full()
            .gap_4()
            .child(
                v_flex()
                    .gap_2()
                    .child(div().text_sm().font_medium().child(crate::tr!("搜索目录", "Search directory")))
                    .child(
                        h_flex()
                            .w_full()
                            .gap_2()
                            .child(
                                div()
                                    .h_8()
                                    .min_w_0()
                                    .flex_1()
                                    .px_3()
                                    .flex()
                                    .items_center()
                                    .rounded(cx.theme().radius)
                                    .border_1()
                                    .border_color(if self.directory_validation_error {
                                        cx.theme().danger
                                    } else {
                                        cx.theme().input
                                    })
                                    .bg(cx.theme().background)
                                    .text_sm()
                                    .text_color(if self.directory.is_some() {
                                        cx.theme().foreground
                                    } else {
                                        cx.theme().muted_foreground
                                    })
                                    .truncate()
                                    .child(path),
                            )
                            .child(
                                Button::new("directory-search-choose-directory")
                                    .small()
                                    .outline()
                                    .icon(IconName::FolderOpen)
                                    .label(if choosing {
                                        crate::tr!("选择中…", "Selecting…")
                                    } else {
                                        crate::tr!("选择…", "Select…")
                                    })
                                    .loading(choosing)
                                    .disabled(choosing)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.choose_directory(window, cx)
                                    })),
                            ),
                    )
                    .when(self.directory_validation_error, |this| {
                        this.child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().danger)
                                .child(crate::tr!("请选择用于搜索的目录", "Select a directory to search")),
                        )
                    }),
            )
            .child(
                v_flex()
                    .gap_2()
                    .child(
                        h_flex()
                            .w_full()
                            .gap_2()
                            .child(
                                Checkbox::new("directory-search-file-type-filter")
                                    .text_sm()
                                    .checked(self.file_type_filter_enabled)
                                    .label(crate::tr!("文件类型", "File types"))
                                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                        this.file_type_filter_enabled = *checked;
                                        if !checked {
                                            this.file_type_validation_error = false;
                                        }
                                        cx.notify();
                                    })),
                            )
                            .child(
                                div().min_w_0().flex_1().child(
                                    Input::new(&self.file_type_patterns)
                                        .small()
                                        .w_full()
                                        .disabled(!self.file_type_filter_enabled)
                                        .aria_label(crate::tr!("文件后缀", "File extensions")),
                                ),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(
                                crate::tr!("多个文件后缀使用 | 分隔，例如 *.txt|*.log；取消勾选时搜索所有文件。", "Separate file extensions with |, for example *.txt|*.log. Clear the checkbox to search all files."),
                            ),
                    )
                    .when(self.file_type_validation_error, |this| {
                        this.child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().danger)
                                .child(crate::tr!("请输入至少一个文件后缀，或取消勾选文件类型", "Enter at least one file extension or clear File types")),
                        )
                    }),
            )
            .child(
                v_flex()
                    .gap_3()
                    .child(
                        Checkbox::new("directory-search-include-subdirectories")
                            .checked(self.include_subdirectories)
                            .label(crate::tr!("包含子目录", "Include subdirectories"))
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.include_subdirectories = *checked;
                                cx.notify();
                            })),
                    )
                    .child(
                        Checkbox::new("directory-search-include-hidden-directories")
                            .checked(self.include_hidden_directories)
                            .label(crate::tr!("包含隐藏目录", "Include hidden directories"))
                            .disabled(!self.include_subdirectories)
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.include_hidden_directories = *checked;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(crate::tr!("目录结果按文件分组显示；打开某条结果时才会创建对应日志标签。", "Directory results are grouped by file. A log tab is created only when a result is opened.")),
            )
    }
}
