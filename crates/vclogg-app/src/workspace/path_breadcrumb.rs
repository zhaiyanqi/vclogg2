use super::*;
use gpui_component::clipboard::Clipboard;

impl Workspace {
    pub(super) fn render_path_breadcrumb(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(tab) = self.active_document() else {
            return h_flex()
                .flex_1()
                .min_w_0()
                .gap_2()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(Icon::new(IconName::File).xsmall())
                .child(crate::tr!("未打开文件", "No file open"))
                .into_any_element();
        };
        let path = tab.document.path();
        // Keep native paths in callbacks; lossy text is only for presentation.
        // ancestors() also keeps Windows drive and UNC roots intact.
        let mut paths = path
            .ancestors()
            .filter(|path| !path.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .collect::<Vec<_>>();
        paths.reverse();
        let menu_paths = paths.clone();
        let workspace = cx.entity();
        let menu_workspace = workspace.clone();
        let show_full_path = self.app_settings.show_full_path;
        let document_id = tab.id;
        let scroll_state = window.use_keyed_state(
            (
                if show_full_path {
                    "toolbar-breadcrumb-full-scroll"
                } else {
                    "toolbar-breadcrumb-compact-scroll"
                },
                document_id,
            ),
            cx,
            |_, _| {
                let scroll = ScrollHandle::new();
                // Reveal the filename on first layout, then leave the user's
                // horizontal position intact on subsequent frames.
                let last_child = if show_full_path {
                    paths.len().saturating_sub(1) * 2
                } else {
                    0
                };
                scroll.scroll_to_item(last_child);
                scroll
            },
        );
        let scroll = scroll_state.read(cx).clone();
        let mut trail = h_flex()
            .id(("toolbar-breadcrumb-trail", document_id))
            .min_w_0()
            .flex_1()
            .overflow_x_scroll()
            .track_scroll(&scroll)
            .gap_1()
            .px_1();
        let visible_paths = paths
            .iter()
            .filter(|item| show_full_path || item.as_path() == path);
        for (ix, item_path) in visible_paths.enumerate() {
            let is_file = item_path.as_path() == path;
            let directory = if is_file {
                item_path.parent().unwrap_or(Path::new("."))
            } else {
                item_path.as_path()
            };
            let directory = if directory.as_os_str().is_empty() {
                PathBuf::from(".")
            } else {
                directory.to_path_buf()
            };
            let click_directory = directory.clone();
            let context_workspace = workspace.clone();
            let label = if is_file && !show_full_path {
                tab.file.title.to_string()
            } else {
                item_path.file_name().map_or_else(
                    || item_path.display().to_string(),
                    |name| name.to_string_lossy().into_owned(),
                )
            };
            if ix > 0 {
                trail = trail.child(
                    Icon::new(IconName::ChevronRight)
                        .xsmall()
                        .text_color(cx.theme().muted_foreground),
                );
            }
            // The dependency's BreadcrumbItem has no context-menu or keyboard
            // command seam. Compose standard Buttons and menus for both inputs.
            let id: SharedString = format!("toolbar-breadcrumb-{item_path:?}").into();
            trail = trail.child(
                div()
                    .id(id.clone())
                    .flex_shrink_0()
                    .child(
                        crate::button_accessibility::with_label(
                            Button::new(id).small().ghost(),
                            label.clone(),
                        )
                        // Button::label clips glyphs to a one-em line box.
                        // Keep the normal button size and font, but give the
                        // text its own line box with room for glyph extents.
                        .child(div().line_height(relative(1.25)).child(label))
                        .text_color(if is_file {
                            cx.theme().foreground
                        } else {
                            cx.theme().muted_foreground
                        })
                        .disabled(self.open_task.is_some())
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.open_files_in_directory(click_directory.clone(), window, cx);
                            },
                        )),
                    )
                    .context_menu(move |menu, window, cx| {
                        Self::build_breadcrumb_menu(
                            menu,
                            directory.clone(),
                            &context_workspace,
                            window,
                            cx,
                        )
                    }),
            );
        }

        h_flex()
            .flex_1()
            .min_w_0()
            .gap_1()
            .child(
                crate::button_accessibility::with_label(
                    Button::new("toolbar-breadcrumb-actions")
                        .small()
                        .ghost()
                        .icon(IconName::FolderOpen)
                        .tooltip(crate::tr!("路径操作", "Path actions")),
                    crate::tr!("路径操作", "Path actions"),
                )
                .dropdown_menu(move |mut menu, window, cx| {
                    for item_path in &menu_paths {
                        let directory = if item_path == menu_paths.last().unwrap() {
                            item_path.parent().unwrap_or(Path::new("."))
                        } else {
                            item_path.as_path()
                        };
                        let directory = if directory.as_os_str().is_empty() {
                            PathBuf::from(".")
                        } else {
                            directory.to_path_buf()
                        };
                        let workspace = menu_workspace.clone();
                        menu = menu.submenu(
                            item_path.display().to_string(),
                            window,
                            cx,
                            move |menu, window, cx| {
                                Self::build_breadcrumb_menu(
                                    menu,
                                    directory.clone(),
                                    &workspace,
                                    window,
                                    cx,
                                )
                            },
                        );
                    }
                    menu
                }),
            )
            .child(trail)
            .child(
                Clipboard::new(("copy-toolbar-file-path", document_id))
                    .value(path.display().to_string())
                    .tooltip(crate::tr!("复制文件路径", "Copy file path")),
            )
            .into_any_element()
    }

    fn build_breadcrumb_menu(
        menu: PopupMenu,
        directory: PathBuf,
        workspace: &Entity<Self>,
        window: &mut Window,
        cx: &App,
    ) -> PopupMenu {
        let open_directory = directory.clone();
        let find_directory = directory;
        Self::popup_menu_with_workspace_action_context(menu, workspace, cx)
            .item(
                PopupMenuItem::new(crate::tr!("打开目录", "Open folder")).on_click(
                    window.listener_for(workspace, move |this, _, window, cx| {
                        match crate::open_directory::launch_custom_directory(
                            &this.app_settings.open_directory_command,
                            &open_directory,
                        ) {
                            Ok(true) => {}
                            Ok(false) => cx.open_with_system(&open_directory),
                            Err(error) => window.notify_message(error.to_string(), cx),
                        }
                    }),
                ),
            )
            .separator()
            .item(
                PopupMenuItem::new(crate::tr!("目录中查找", "Find in folder")).on_click(
                    window.listener_for(workspace, move |this, _, window, cx| {
                        this.find_in_directory(find_directory.clone(), window, cx);
                    }),
                ),
            )
    }
}
