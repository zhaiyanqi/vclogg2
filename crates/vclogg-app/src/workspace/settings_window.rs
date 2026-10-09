use super::*;
use gpui_kit::{KeyBinding, WindowBounds, WindowOptions};

#[cfg(target_os = "macos")]
mod macos;

const ENTER_DURATION: Duration = Duration::from_millis(190);
const ENTER_FADE_DURATION: Duration = Duration::from_millis(160);
const ENTER_OFFSET: f32 = 8.;

#[derive(Clone, Copy)]
enum Entrance {
    Idle,
    AwaitingFrame,
    Running(std::time::Instant),
}

const SETTINGS_CONTEXT: &str = "VCLogg2Settings";
gpui_kit::actions!(settings_window, [CloseSettings, SaveSettings]);

pub(super) fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("secondary-w", CloseSettings, Some(SETTINGS_CONTEXT)),
        KeyBinding::new("escape", CloseSettings, Some(SETTINGS_CONTEXT)),
        KeyBinding::new("secondary-s", SaveSettings, Some(SETTINGS_CONTEXT)),
    ]);
}

/// Owns the editing session independently of the originating native window.
/// The retained workspace keeps the existing preference services alive if its window closes.
pub(super) struct SettingsWindow {
    workspace: Entity<Workspace>,
    settings: Entity<SettingsDialog>,
    original: AppSettings,
    last_draft: AppSettings,
    last_preview: AppSettings,
    original_history: Vec<String>,
    highlight_baseline: crate::color_labels_dialog::LogColoringConfig,
    focus: FocusHandle,
    saving: bool,
    enter_opacity: f32,
    enter_progress: f32,
    entrance: Entrance,

    pub(super) save_task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl Workspace {
    pub(super) fn open_settings_dialog(
        &mut self,
        requested_category: Option<SettingsCategory>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let workspace = cx.entity();
        let display = window.display(cx);
        let display_id = display.as_ref().map(|display| display.id());
        let preferred = self
            .settings_dialog_geometry
            .preferred_size
            .unwrap_or_else(|| size(window.rem_size() * 80., window.rem_size() * 50.));
        let preferred = display.as_ref().map_or(preferred, |display| {
            let available = display.bounds().size;
            size(
                preferred
                    .width
                    .max(px(720.))
                    .min((available.width - px(48.)).max(px(720.))),
                preferred
                    .height
                    .max(px(480.))
                    .min((available.height - px(96.)).max(px(480.))),
            )
        });
        // Opening a second window must happen after the caller releases its Workspace borrow.
        cx.defer(move |cx| {
            if let Some((handle, view)) = cx
                .global::<WorkspaceWindowRegistry>()
                .settings_window
                .clone()
                && handle
                    .update(cx, |_, window, cx| {
                        if let Some(category) = requested_category {
                            _ = view.update(cx, |view, cx| {
                                view.settings.update(cx, |settings, cx| {
                                    settings.select_category(category, window, cx)
                                });
                            });
                        }
                        window.activate_window();
                    })
                    .is_ok()
            {
                return;
            }
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    display_id, preferred, cx,
                ))),
                display_id,
                show: false,
                focus: false,
                window_min_size: Some(size(px(720.), px(480.))),
                ..TitleBar::window_options()
            };
            #[cfg(target_os = "macos")]
            let (options, traffic_light_position) =
                crate::macos_window_controls::configure(options);
            match gpui_kit::open_window(options, cx, |window, cx| {
                window.set_window_title(crate::tr!("设置", "Settings"));
                crate::app_icon::attach_window(window, cx);
                #[cfg(target_os = "macos")]
                if let Some(position) = traffic_light_position
                    && let Err(error) = crate::macos_window_controls::attach(window, position)
                {
                    log::error!("Could not initialize settings window controls: {error:#}");
                }
                cx.new(|cx| SettingsWindow::new(workspace.clone(), requested_category, window, cx))
            }) {
                Ok((handle, view)) => {
                    cx.global_mut::<WorkspaceWindowRegistry>().settings_window =
                        Some((handle, view.downgrade()));
                    // GPUI's open_window builds and draws the root before returning.
                    // Reveal only after that work, without waiting for display-link
                    // callbacks: macOS suspends those while a native window is hidden.
                    _ = handle.update(cx, |_, window, cx| {
                        view.update(cx, |view, cx| view.reveal(window, cx));
                    });
                }
                Err(error) => log::error!("Could not open settings window: {error:#}"),
            }
        });
    }
}

impl SettingsWindow {
    fn new(
        workspace: Entity<Workspace>,
        requested: Option<SettingsCategory>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let original = workspace.read(cx).app_settings.clone();
        let original_history = workspace.read(cx).search_history.clone();
        let highlight_baseline = workspace.read(cx).highlight_config();
        let category = requested
            .filter(|category| category.is_available())
            .unwrap_or(
                cx.global::<WorkspaceWindowRegistry>()
                    .last_settings_category,
            );
        let network = {
            let owner = workspace.read(cx);
            SettingsNetworkSnapshot {
                settings: owner.cloud.settings.clone(),
                client: owner.cloud.client.clone(),
                connection: owner.cloud.connection.clone(),
                client_error: owner.cloud.client_error.clone(),
            }
        };
        let settings = cx.new(|cx| {
            SettingsDialog::new(
                original.clone(),
                original_history.clone(),
                network,
                category,
                window,
                cx,
            )
            .with_highlight_baseline(highlight_baseline.clone(), window, cx)
        });
        workspace.update(cx, |owner, cx| {
            owner.load_settings_history(&settings, window, cx);
            owner.remember_settings_category(category, window, cx);
        });
        let subscription = cx.subscribe_in(
            &settings,
            window,
            |this, _, event: &SettingsDialogEvent, window, cx| match event {
                SettingsDialogEvent::DraftChanged => this.preview(window, cx),
                SettingsDialogEvent::CategoryChanged(category) => {
                    if *category == SettingsCategory::Highlight {
                        this.settings.update(cx, |settings, cx| {
                            settings.ensure_highlight_editor(window, cx)
                        });
                    }
                    this.workspace.update(cx, |owner, cx| {
                        owner.remember_settings_category(*category, window, cx)
                    });
                }
                SettingsDialogEvent::CloudSettings(settings) => {
                    this.workspace.update(cx, |owner, cx| {
                        owner.save_cloud_settings(settings.clone(), window, cx)
                    })
                }
                SettingsDialogEvent::CloudConnection(connection) => {
                    this.workspace.update(cx, |owner, cx| {
                        owner.cloud.connection = connection.clone();
                        cx.notify();
                    })
                }
            },
        );
        let observer = cx.observe(&workspace, |_, _, cx| cx.notify());
        let weak = cx.weak_entity();
        window.on_window_should_close(cx, move |window, cx| {
            weak.update(cx, |this, cx| this.cancel(window, cx))
                .unwrap_or(true)
        });
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        // The first hidden-window frame must already match the entrance start;
        // otherwise showing the window can flash the fully visible page first.
        let enter_opacity = if original.reduce_motion { 1. } else { 0. };
        let enter_progress = if original.reduce_motion { 1. } else { 0. };
        Self {
            workspace,
            settings,
            original: original.clone(),
            last_draft: original.clone(),
            last_preview: original,
            original_history,
            highlight_baseline,
            focus,
            saving: false,
            enter_opacity,
            enter_progress,
            entrance: Entrance::Idle,

            save_task: None,
            _subscriptions: vec![subscription, observer],
        }
    }

    fn reveal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Own this transition rather than relying on the OS window animation
        // preference. The explicit in-app reduced-motion setting still applies.
        let animate = !self.original.reduce_motion;
        self.enter_opacity = if animate { 0. } else { 1. };
        self.enter_progress = if animate { 0. } else { 1. };
        #[cfg(target_os = "macos")]
        macos::prepare(window);
        window.activate_window();
        if !animate {
            return;
        }
        // All platforms share frame timing, easing and reveal geometry.
        // Native window decorations remain owned by their platform.
        // Activation can be asynchronous and the first visible frame can be
        // delayed. Do not spend the animation duration while waiting for it.
        self.entrance = Entrance::AwaitingFrame;
        Self::entrance_frame(cx.weak_entity(), window);
        cx.notify();
    }

    fn entrance_frame(view: WeakEntity<Self>, window: &Window) {
        window.on_next_frame(move |window, cx| {
            if !window.is_visible() {
                Self::entrance_frame(view, window);
                return;
            }
            let pending = view
                .update(cx, |this, cx| {
                    let started = match this.entrance {
                        Entrance::Idle => return false,
                        Entrance::AwaitingFrame => {
                            let now = std::time::Instant::now();
                            this.entrance = Entrance::Running(now);
                            now
                        }
                        Entrance::Running(started) => started,
                    };
                    let elapsed = started.elapsed();
                    let fade = (elapsed.as_secs_f32() / ENTER_FADE_DURATION.as_secs_f32()).min(1.);
                    let progress = (elapsed.as_secs_f32() / ENTER_DURATION.as_secs_f32()).min(1.);
                    this.enter_opacity = ease_out_cubic(fade);
                    this.enter_progress = ease_out_cubic(progress);
                    if progress >= 1. {
                        this.entrance = Entrance::Idle;
                    }
                    cx.notify();
                    !matches!(this.entrance, Entrance::Idle)
                })
                .unwrap_or(false);
            if pending {
                Self::entrance_frame(view, window);
            }
        });
    }

    /// Persist a log-window command without accidentally committing the settings preview.
    pub(super) fn committed_settings(settings: AppSettings, cx: &App) -> AppSettings {
        let Some(view) = cx
            .global::<WorkspaceWindowRegistry>()
            .settings_window
            .as_ref()
            .and_then(|(_, view)| view.upgrade())
        else {
            return settings;
        };
        let view = view.read(cx);
        merge_changes(&view.last_preview, &settings, &view.original)
    }

    fn sync_retained_workspace(&self, cx: &mut App) {
        let registry = cx.global::<WorkspaceWindowRegistry>();
        if registry
            .windows
            .iter()
            .any(|entry| entry.workspace == self.workspace)
        {
            return;
        }
        let Some(source) = registry
            .windows_by_recent_focus()
            .first()
            .map(|entry| entry.workspace.clone())
        else {
            return;
        };
        let settings = source.read(cx).app_settings.clone();
        let labels = source.read(cx).color_labels.clone();
        let history = source.read(cx).search_history.clone();
        self.workspace.update(cx, |owner, _| {
            owner.app_settings = settings;
            owner.color_labels = labels;
            owner.search_history = history;
        });
    }

    fn busy(&self, cx: &App) -> bool {
        let workspace = self.workspace.read(cx);
        self.saving || workspace.settings_saving || workspace.color_labels_saving
    }

    fn preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy(cx) {
            return;
        }
        let Ok(draft) = self.settings.read(cx).settings(cx) else {
            return;
        };
        self.sync_retained_workspace(cx);
        let current = self.workspace.read(cx).app_settings.clone();
        // Rebase rollback values when the log window changed a setting independently.
        self.original = merge_changes(&self.last_preview, &current, &self.original);
        let next = merge_changes(&self.last_draft, &draft, &current);
        self.last_draft = draft;
        self.workspace
            .update(cx, |owner, cx| owner.preview_app_settings(next, window, cx));
        self.last_preview = self.workspace.read(cx).app_settings.clone();
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy(cx) {
            return;
        }
        if self.workspace.read(cx).persistence.store.is_none() {
            window.notify_message(
                crate::tr!(
                    "状态库尚未就绪，请稍后重试",
                    "Storage is not ready. Try again shortly."
                ),
                cx,
            );
            return;
        }
        self.settings.update(cx, |settings, cx| {
            settings.ensure_highlight_editor(window, cx)
        });
        let Some(editor) = self.settings.read(cx).highlight_editor() else {
            return;
        };
        if let Err(error) = editor.read(cx).config(cx) {
            window.notify_message(error, cx);
            return;
        }
        let (draft, history, network) = {
            let settings = self.settings.read(cx);
            let draft = match settings.settings(cx) {
                Ok(draft) => draft,
                Err(error) => {
                    window.notify_message(error, cx);
                    return;
                }
            };
            (
                draft,
                settings.search_history(),
                settings.network_settings(cx),
            )
        };
        self.sync_retained_workspace(cx);
        let current = self.workspace.read(cx).app_settings.clone();
        let draft = merge_changes(&self.last_draft, &draft, &current);
        let removed = self
            .original_history
            .iter()
            .filter(|query| !history.contains(query))
            .cloned()
            .collect::<Vec<_>>();
        self.saving = true;
        self.focus.focus(window, cx);
        let (settings_task, cloud_task, history_task) = self.workspace.update(cx, |owner, cx| {
            owner.save_app_settings(draft, window, cx);
            owner.save_cloud_settings(network, window, cx);
            owner.remove_search_history_entries(&removed, window, cx);
            owner.save_highlight_settings_dialog(
                editor,
                self.highlight_baseline.clone(),
                window,
                cx,
            );
            (
                owner.persistence.app_settings_save_task.take(),
                owner.persistence.state_tasks.pop(),
                owner.persistence.search_history_save_task.take(),
            )
        });
        self.save_task = Some(cx.spawn_in(window, async move |this, cx| {
            for task in [settings_task, cloud_task, history_task]
                .into_iter()
                .flatten()
            {
                task.await;
            }
            _ = this.update_in(cx, |this, window, cx| {
                this.saving = false;
                if !this.workspace.read(cx).settings_save_failed {
                    Self::finish_saved(window, cx);
                } else if let Some(editor) = this.settings.read(cx).highlight_editor() {
                    editor.update(cx, |editor, cx| {
                        editor.save_failed(
                            crate::tr!(
                                "设置未能全部保存，请重试",
                                "Some settings could not be saved. Try again."
                            )
                            .to_string(),
                            cx,
                        )
                    });
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn persist_size(&self, window: &Window, cx: &mut App) {
        let extent = window.viewport_size();
        self.workspace.update(cx, |owner, cx| {
            owner.settings_dialog_geometry.preferred_size = Some(extent);
            if let Some(store) = owner.persistence.store.clone() {
                let task = cx.background_spawn(async move {
                    if let Err(error) = store
                        .save_settings_dialog_size([extent.width.as_f32(), extent.height.as_f32()])
                    {
                        log::warn!("Could not save settings window size: {error:#}");
                    }
                });
                cx.update_global::<WorkspaceWindowRegistry, _>(|registry, cx| {
                    registry.closed_flush_tasks.push(task, cx);
                });
            }
        });
    }

    fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.busy(cx) {
            return false;
        }
        self.sync_retained_workspace(cx);
        let current = self.workspace.read(cx).app_settings.clone();
        // Start with the original, then keep every external change since our last preview.
        let restored = merge_changes(&self.last_preview, &current, &self.original);
        self.workspace.update(cx, |owner, cx| {
            owner.preview_app_settings(restored, window, cx)
        });
        self.persist_size(window, cx);
        cx.global_mut::<WorkspaceWindowRegistry>().settings_window = None;
        true
    }

    pub(super) fn refresh_history(window: &Window, cx: &mut App) {
        let Some((handle, view)) = cx
            .global::<WorkspaceWindowRegistry>()
            .settings_window
            .clone()
        else {
            return;
        };
        if handle != window.window_handle() {
            return;
        }
        cx.defer(move |cx| {
            _ = handle.update(cx, |_, window, cx| {
                _ = view.update(cx, |view, cx| {
                    view.workspace.update(cx, |owner, cx| {
                        owner.load_settings_history(&view.settings, window, cx)
                    });
                });
            });
        });
    }

    pub(super) fn finish_saved(window: &mut Window, cx: &mut App) -> bool {
        let Some((handle, view)) = cx
            .global::<WorkspaceWindowRegistry>()
            .settings_window
            .clone()
        else {
            return false;
        };
        if handle != window.window_handle() {
            return false;
        }
        // Save completion runs inside Workspace::update; release it before touching the session.
        cx.defer(move |cx| {
            _ = handle.update(cx, |_, window, cx| {
                _ = view.update(cx, |this, cx| this.persist_size(window, cx));
                cx.global_mut::<WorkspaceWindowRegistry>().settings_window = None;
                window.remove_window();
            });
        });
        true
    }
}

impl Render for SettingsWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        window.set_window_title(crate::tr!("设置", "Settings"));
        let busy = self.busy(cx);
        let extent = window.viewport_size();
        let offset = px(ENTER_OFFSET * (1. - self.enter_progress));
        let content = v_flex()
            .id("settings-window-content")
            .w(extent.width)
            .h(extent.height)
            .absolute()
            .left_0()
            .top(offset)
            .opacity(self.enter_opacity)
            .min_h_0()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .key_context(SETTINGS_CONTEXT)
            .track_focus(&self.focus)
            .when(busy, |surface| {
                surface
                    .capture_any_mouse_down(|_, window, cx| {
                        window.prevent_default();
                        cx.stop_propagation();
                    })
                    .capture_key_down(|_, window, cx| {
                        window.prevent_default();
                        cx.stop_propagation();
                    })
            })
            .on_action(cx.listener(|this, _: &CloseSettings, window, cx| {
                if this.cancel(window, cx) {
                    window.remove_window();
                }
            }))
            .on_action(cx.listener(|this, _: &OpenSettings, window, cx| {
                if this.cancel(window, cx) {
                    window.remove_window();
                }
            }))
            .on_action(cx.listener(|this, _: &SaveSettings, window, cx| this.save(window, cx)))
            .child(
                TitleBar::new()
                    .h(px(36.))
                    .border_b_0()
                    .bg(cx.theme().background)
                    .when(cfg!(target_os = "macos") && window.is_fullscreen(), |bar| {
                        // Keep the native controls' lane, accounting for TitleBar's fullscreen inset.
                        bar.pl(px(80.) - rems(0.75).to_pixels(window.rem_size()))
                    })
                    .on_close_window(cx.listener(|this, _, window, cx| {
                        if this.cancel(window, cx) {
                            window.remove_window();
                        }
                    }))
                    .child(
                        h_flex()
                            .h_full()
                            .flex_1()
                            .items_center()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(crate::tr!("设置", "Settings")),
                    ),
            )
            .child(div().flex_1().min_h_0().p_4().child(self.settings.clone()))
            .child(
                h_flex()
                    .flex_shrink_0()
                    .px_4()
                    .pb_4()
                    .justify_between()
                    .gap_2()
                    .child(
                        Button::new("settings-window-reset")
                            .outline()
                            .disabled(busy)
                            .label(crate::tr!("恢复默认设置", "Restore default settings"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.workspace.update(cx, |owner, cx| {
                                    owner.confirm_reset_app_settings(
                                        this.original.clone(),
                                        window,
                                        cx,
                                    )
                                });
                            })),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("settings-window-cancel")
                                    .disabled(busy)
                                    .label(crate::tr!("取消", "Cancel"))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        if this.cancel(window, cx) {
                                            window.remove_window();
                                        }
                                    })),
                            )
                            .child(
                                Button::new("settings-window-save")
                                    .primary()
                                    .disabled(busy)
                                    .loading(busy)
                                    .label(crate::tr!("保存", "Save"))
                                    .on_click(
                                        cx.listener(|this, _, window, cx| this.save(window, cx)),
                                    ),
                            ),
                    ),
            );
        // Move the fixed-size content as a unit. Unlike the old centered crop,
        // this is visible motion without changing layout or wrapping text.
        div()
            .id("settings-window-entrance")
            .size_full()
            .relative()
            .overflow_hidden()
            .child(content)
    }
}

/// Apply only fields edited between two snapshots, preserving unrelated concurrent edits.
fn merge_changes(before: &AppSettings, after: &AppSettings, current: &AppSettings) -> AppSettings {
    let mut merged = current.clone();
    macro_rules! fields {
        ($($field:ident),* $(,)?) => { $(if before.$field != after.$field { merged.$field.clone_from(&after.$field); })* };
    }
    fields!(
        app_icon,
        app_log_level,
        language,
        theme_preference,
        default_show_line_numbers,
        default_show_row_separators,
        show_line_number_row_separators,
        line_number_width,
        line_number_text_color,
        line_number_background_color,
        light_log_text_color,
        light_log_background_color,
        dark_log_text_color,
        dark_log_background_color,
        highlight_log_levels,
        log_coloring,
        keyword_match_styles,
        selection_styles,
        log_font_size,
        search_toolbar_height,
        search_toolbar_font_size,
        search_input_font_size,
        log_line_spacing,
        log_font_family,
        mouse_wheel_scroll_percent,
        scroll_by_line,
        mouse_wheel_scroll_lines,
        scroll_by_line_when_word_wrap,
        viewer_overscan,
        reduce_motion,
        confirm_close_tab,
        confirm_clipboard_paste,
        show_full_path,
        show_horizontal_scrollbar,
        open_directory_command,
        max_search_results,
        highlight_matches,
        word_boundary_characters,
        default_case_sensitive,
        default_use_regex,
        shortcuts,
    );
    merged
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{TestAppContext, component::Root, test::TestWindowExt};

    fn init(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::actions::init(cx);
            Workspace::init_window_registry(cx);
            crate::notifications::init(cx);
            crate::app_icon::init(cx);
            cx.set_global(crate::system_fonts::SystemFonts::new(Arc::new(
                gpui_kit::NoopTextSystem,
            )));
        });
    }

    fn workspace(
        cx: &mut TestAppContext,
        store: Arc<StateStore>,
    ) -> (AnyWindowHandle, Entity<Workspace>) {
        let mut owner = None;
        let window = cx.add_window(|window, cx| {
            let view = cx.new(|cx| Workspace::new(false, Vec::new(), window, cx));
            view.update(cx, |view, _| {
                view.persistence._bootstrap_task = Task::ready(());
                view.persistence.store = Some(store);
            });
            Workspace::register_window(&view, window, cx);
            owner = Some(view.clone());
            Root::new(view, window, cx)
        });
        (window.into(), owner.unwrap())
    }

    fn open(
        cx: &mut TestAppContext,
        handle: AnyWindowHandle,
        owner: &Entity<Workspace>,
    ) -> (AnyWindowHandle, Entity<SettingsWindow>) {
        cx.update_window(handle, |_, window, cx| {
            owner.update(cx, |owner, cx| owner.open_settings_dialog(None, window, cx))
        })
        .unwrap();
        cx.run_until_parked();
        cx.update(|cx| {
            let (handle, view) = cx
                .global::<WorkspaceWindowRegistry>()
                .settings_window
                .clone()
                .unwrap();
            (handle, view.upgrade().unwrap())
        })
    }

    #[gpui_kit::test]
    fn settings_window_entrance_preserves_layout_and_ignores_global_motion_policy(
        cx: &mut TestAppContext,
    ) {
        init(cx);
        let directory = tempfile::tempdir().unwrap();
        let store = Arc::new(StateStore::open(directory.path().join("state.db")).unwrap());
        let (main, owner) = workspace(cx, store);
        cx.update(|cx| cx.set_reduce_motion(true));
        let (settings, view) = open(cx, main, &owner);
        cx.update(|cx| {
            assert_eq!(cx.active_window(), Some(settings));
            assert_eq!(view.read(cx).enter_opacity, 0.);
        });
        assert_eq!(open(cx, main, &owner).0, settings);
        cx.update(|cx| assert!(matches!(view.read(cx).entrance, Entrance::AwaitingFrame)));
        cx.update_window(settings, |_, window, cx| {
            window.simulate_next_frame(cx);
            assert!(matches!(view.read(cx).entrance, Entrance::Running(_)));
            assert!(view.read(cx).enter_opacity < 0.1);
            assert!(view.read(cx).enter_progress < 0.1);
        })
        .unwrap();
        let initial_button = cx
            .update_window(settings, |_, window, cx| {
                window.render_frame(cx);
                window.find("settings-window-save").bounds()
            })
            .unwrap();
        cx.update(|cx| {
            view.update(cx, |view, _| {
                view.entrance =
                    Entrance::Running(std::time::Instant::now() - Duration::from_millis(60));
            })
        });
        cx.update_window(settings, |_, window, cx| {
            window.simulate_next_frame(cx);
            window.render_frame(cx);
            let current = window.find("settings-window-save").bounds();
            assert_eq!(initial_button.size, current.size);
            assert!(current.origin.y < initial_button.origin.y);
            assert!(current.origin.y > initial_button.origin.y - px(ENTER_OFFSET));
            assert!((0.0..1.0).contains(&view.read(cx).enter_opacity));
        })
        .unwrap();
        cx.update(|cx| {
            view.update(cx, |view, _| {
                view.entrance = Entrance::Running(std::time::Instant::now() - ENTER_DURATION);
            })
        });
        cx.update_window(settings, |_, window, cx| {
            window.simulate_next_frame(cx);
            window.render_frame(cx);
            let final_button = window.find("settings-window-save").bounds();
            assert_eq!(initial_button.size, final_button.size);
            assert!((initial_button.origin.x - final_button.origin.x).abs() <= px(1.));
            assert!(
                (initial_button.origin.y - final_button.origin.y - px(ENTER_OFFSET)).abs()
                    <= px(1.)
            );
        })
        .unwrap();
        cx.update(|cx| {
            assert_eq!(view.read(cx).enter_opacity, 1.);
            assert_eq!(view.read(cx).enter_progress, 1.);
            assert!(matches!(view.read(cx).entrance, Entrance::Idle));
        });
    }

    #[gpui_kit::test]
    fn settings_window_activates_without_hidden_window_frame_callbacks(cx: &mut TestAppContext) {
        init(cx);
        let directory = tempfile::tempdir().unwrap();
        let store = Arc::new(StateStore::open(directory.path().join("state.db")).unwrap());
        let (main, owner) = workspace(cx, store);
        let (settings, _view) = open(cx, main, &owner);
        // The test platform does not deliver native display-link callbacks to hidden windows.
        cx.update(|cx| assert_eq!(cx.active_window(), Some(settings)));
        cx.update_window(main, |_, window, _| window.activate_window())
            .unwrap();
        assert_eq!(open(cx, main, &owner).0, settings);
        cx.update(|cx| assert_eq!(cx.active_window(), Some(settings)));
    }

    #[gpui_kit::test]
    fn settings_window_is_singleton_and_cancel_preserves_log_window_changes(
        cx: &mut TestAppContext,
    ) {
        init(cx);
        let directory = tempfile::tempdir().unwrap();
        let store = Arc::new(StateStore::open(directory.path().join("state.db")).unwrap());
        let (main, owner) = workspace(cx, store.clone());
        let (other, other_owner) = workspace(cx, store.clone());
        let (settings, view) = open(cx, main, &owner);
        assert_ne!(main, settings);
        assert_eq!(open(cx, other, &other_owner).0, settings);
        cx.update_window(settings, |_, window, cx| {
            window.render_frame(cx);
            window.click("settings-show-full-path", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update(|cx| {
            assert!(!owner.read(cx).app_settings.show_full_path);
            assert!(!other_owner.read(cx).app_settings.show_full_path);
        });
        cx.update_window(main, |_, window, cx| {
            owner.update(cx, |owner, cx| {
                owner.update_app_setting(|settings| settings.log_font_size = 20, window, cx);
            })
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(settings, |_, window, cx| {
            window.render_frame(cx);
            window.click("settings-window-cancel", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update(|cx| {
            assert!(owner.read(cx).app_settings.show_full_path);
            assert_eq!(owner.read(cx).app_settings.log_font_size, 20);
            assert!(other_owner.read(cx).app_settings.show_full_path);
            assert!(
                cx.global::<WorkspaceWindowRegistry>()
                    .settings_window
                    .is_none()
            );
        });
        let persisted = store.load_app_settings().unwrap();
        assert!(
            persisted.show_full_path,
            "a log-window command must not persist the preview"
        );
        assert_eq!(persisted.log_font_size, 20);
        drop(view);
    }

    #[gpui_kit::test]
    fn settings_window_saves_after_source_window_closes(cx: &mut TestAppContext) {
        init(cx);
        let directory = tempfile::tempdir().unwrap();
        let store = Arc::new(StateStore::open(directory.path().join("state.db")).unwrap());
        let (main, owner) = workspace(cx, store.clone());
        let (settings, _view) = open(cx, main, &owner);
        cx.update_window(main, |_, window, cx| {
            Workspace::unregister_window(window.window_handle().window_id(), cx);
            window.remove_window();
        })
        .unwrap();
        cx.update_window(settings, |_, window, cx| {
            window.render_frame(cx);
            window.click("settings-show-full-path", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(settings, |_, window, cx| {
            window.render_frame(cx);
            window.click("settings-window-save", cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert!(!store.load_app_settings().unwrap().show_full_path);
        cx.update(|cx| {
            assert!(
                cx.global::<WorkspaceWindowRegistry>()
                    .settings_window
                    .is_none()
            )
        });
    }

    #[gpui_kit::test]
    fn missing_storage_keeps_draft_for_retry_and_blocks_busy_close(cx: &mut TestAppContext) {
        init(cx);
        let directory = tempfile::tempdir().unwrap();
        let store = Arc::new(StateStore::open(directory.path().join("state.db")).unwrap());
        let (main, owner) = workspace(cx, store.clone());
        let (settings, view) = open(cx, main, &owner);
        cx.update_window(settings, |_, window, cx| {
            window.render_frame(cx);
            window.click("settings-show-full-path", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update(|cx| owner.update(cx, |owner, _| owner.persistence.store = None));
        cx.update_window(settings, |_, window, cx| {
            window.render_frame(cx);
            window.click("settings-window-save", cx);
            assert!(
                cx.global::<WorkspaceWindowRegistry>()
                    .settings_window
                    .is_some()
            );
            assert!(
                !view
                    .read(cx)
                    .settings
                    .read(cx)
                    .settings(cx)
                    .unwrap()
                    .show_full_path
            );
            view.update(cx, |view, cx| {
                view.saving = true;
                assert!(!view.cancel(window, cx));
                view.saving = false;
            });
        })
        .unwrap();
        cx.update(|cx| owner.update(cx, |owner, _| owner.persistence.store = Some(store.clone())));
        cx.update_window(settings, |_, window, cx| {
            window.render_frame(cx);
            window.press("secondary-s", cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert!(!store.load_app_settings().unwrap().show_full_path);
        cx.update(|cx| {
            assert!(
                cx.global::<WorkspaceWindowRegistry>()
                    .settings_window
                    .is_none()
            )
        });
    }

    #[gpui_kit::test]
    fn settings_window_escape_rolls_back_preview(cx: &mut TestAppContext) {
        init(cx);
        let directory = tempfile::tempdir().unwrap();
        let store = Arc::new(StateStore::open(directory.path().join("state.db")).unwrap());
        let (main, owner) = workspace(cx, store);
        let (settings, _view) = open(cx, main, &owner);
        cx.update_window(settings, |_, window, cx| {
            window.render_frame(cx);
            window.click("settings-show-full-path", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(settings, |_, window, cx| {
            window.press("escape", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update(|cx| {
            assert!(owner.read(cx).app_settings.show_full_path);
            assert!(
                cx.global::<WorkspaceWindowRegistry>()
                    .settings_window
                    .is_none()
            );
        });
    }

    #[gpui_kit::test]
    fn settings_shortcut_toggles_window_and_rolls_back_preview(cx: &mut TestAppContext) {
        init(cx);
        let directory = tempfile::tempdir().unwrap();
        let store = Arc::new(StateStore::open(directory.path().join("state.db")).unwrap());
        let (main, owner) = workspace(cx, store);
        let defaults = ShortcutSettings::default();
        let mut shortcuts = defaults.clone();
        for binding in [defaults.open_settings.as_str(), "Ctrl+Alt+,"] {
            let previous = shortcuts.clone();
            shortcuts.open_settings = binding.to_owned();
            cx.update(|cx| crate::actions::apply_shortcuts(&previous, &shortcuts, cx));
            let key = crate::actions::shortcut_to_key_binding(binding).unwrap();
            cx.update_window(main, |_, window, cx| {
                window.activate_window();
                window.render_frame(cx);
                window.press(&key, cx);
            })
            .unwrap();
            cx.run_until_parked();
            let (settings, view) = cx.update(|cx| {
                let (handle, view) = cx
                    .global::<WorkspaceWindowRegistry>()
                    .settings_window
                    .clone()
                    .expect("shortcut opens settings");
                (handle, view.upgrade().unwrap())
            });
            cx.update_window(settings, |_, window, cx| {
                window.render_frame(cx);
                window.click("settings-show-full-path", cx);
            })
            .unwrap();
            cx.run_until_parked();
            cx.update_window(settings, |_, window, cx| {
                view.update(cx, |view, _| view.saving = true);
                window.render_frame(cx);
                window.press(&key, cx);
                assert!(
                    cx.global::<WorkspaceWindowRegistry>()
                        .settings_window
                        .is_some()
                );
                view.update(cx, |view, _| view.saving = false);
                window.render_frame(cx);
                window.press(&key, cx);
            })
            .unwrap();
            cx.run_until_parked();
            cx.update(|cx| {
                assert!(
                    cx.global::<WorkspaceWindowRegistry>()
                        .settings_window
                        .is_none()
                );
                assert!(owner.read(cx).app_settings.show_full_path);
            });
        }
    }

    #[test]
    fn merging_draft_changes_keeps_unrelated_concurrent_values() {
        let before = AppSettings::default();
        let after = AppSettings {
            show_full_path: false,
            ..before.clone()
        };
        let concurrent = AppSettings {
            log_font_size: 20,
            ..before.clone()
        };
        let merged = merge_changes(&before, &after, &concurrent);
        assert!(!merged.show_full_path);
        assert_eq!(merged.log_font_size, 20);
        let restored = merge_changes(&after, &merged, &before);
        assert!(restored.show_full_path);
        assert_eq!(restored.log_font_size, 20);
    }
}
