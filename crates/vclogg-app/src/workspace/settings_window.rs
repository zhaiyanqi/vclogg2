use super::*;
use gpui_kit::{KeyBinding, WindowBounds, WindowOptions};

#[cfg(target_os = "macos")]
mod macos;

const SETTINGS_CONTEXT: &str = "VCLogg2Settings";
gpui_kit::actions!(settings_window, [CloseSettings]);

pub(super) fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("secondary-w", CloseSettings, Some(SETTINGS_CONTEXT)),
        KeyBinding::new("escape", CloseSettings, Some(SETTINGS_CONTEXT)),
    ]);
}

/// Owns the editing session independently of the originating native window.
/// The retained workspace keeps the existing preference services alive if its window closes.
pub(super) struct SettingsWindow {
    workspace: Entity<Workspace>,
    settings: Entity<SettingsDialog>,
    saved_draft: AppSettings,
    saved_network: CloudSettings,
    last_draft: AppSettings,
    saved_history: Vec<String>,
    highlight_baseline: crate::color_labels_dialog::LogColoringConfig,
    last_highlight: crate::color_labels_dialog::LogColoringConfig,
    focus: FocusHandle,
    saving: bool,
    save_pending: bool,
    closing: bool,
    quitting: bool,
    save_error: Option<String>,
    validation_error: Option<String>,
    debounce_task: Option<Task<()>>,
    save_task: Option<Task<()>>,
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
                    _ = handle.update(cx, |_, window, _| {
                        #[cfg(target_os = "macos")]
                        macos::disable_animation(window);
                        window.activate_window();
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
        let initial_network = workspace.read(cx).cloud.settings.clone();
        let initial_draft = settings
            .read(cx)
            .settings(cx)
            .unwrap_or_else(|_| original.clone());
        workspace.update(cx, |owner, cx| {
            owner.load_settings_history(&settings, window, cx);
            owner.remember_settings_category(category, window, cx);
        });
        let subscription = cx.subscribe_in(
            &settings,
            window,
            |this, _, event: &SettingsDialogEvent, window, cx| match event {
                SettingsDialogEvent::DraftChanged => this.draft_changed(window, cx),
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
            weak.update(cx, |this, cx| {
                this.request_close(window, cx);
                false
            })
            .unwrap_or(true)
        });
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        Self {
            workspace,
            settings,
            saved_draft: initial_draft.clone(),
            saved_network: initial_network,
            last_draft: initial_draft,
            saved_history: original_history,
            last_highlight: highlight_baseline.clone(),
            highlight_baseline,
            focus,
            saving: false,
            save_pending: false,
            closing: false,
            quitting: false,
            save_error: None,
            validation_error: None,
            debounce_task: None,
            save_task: None,
            _subscriptions: vec![subscription, observer],
        }
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
        self.saving || self.debounce_task.is_some() || workspace.settings_saving
    }

    fn draft_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.quitting {
            return;
        }
        self.sync_retained_workspace(cx);
        {
            let (draft, error) = self
                .settings
                .read(cx)
                .settings_for_autosave(&self.last_draft, cx);
            self.validation_error = error;
            let current = self.workspace.read(cx).app_settings.clone();
            let next = merge_changes(&self.last_draft, &draft, &current);
            if next != current {
                self.workspace
                    .update(cx, |owner, cx| owner.apply_app_settings(next, window, cx));
            }
            self.last_draft = draft;
        }
        if let Some(editor) = self.settings.read(cx).highlight_editor()
            && let Ok(config) = editor.read(cx).config(cx)
        {
            self.last_highlight = config;
        }
        self.last_highlight.highlight_log_levels = self.last_draft.highlight_log_levels;
        self.save_pending = true;
        if !self.saving {
            self.debounce_task = Some(cx.spawn_in(window, async move |this, cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(300))
                    .await;
                _ = this.update_in(cx, |this, window, cx| this.save(window, cx));
            }));
        }
        cx.notify();
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.debounce_task = None;
        if self.saving || self.quitting {
            return;
        }
        let settings = self.settings.read(cx);
        let parsed = settings.settings(cx);
        let highlight = settings
            .highlight_editor()
            .map(|editor| editor.read(cx).config(cx));
        self.validation_error = parsed.as_ref().err().cloned().or_else(|| {
            highlight
                .as_ref()
                .and_then(|result| result.as_ref().err().cloned())
        });
        let draft = (self.last_draft != self.saved_draft).then(|| self.last_draft.clone());
        let network = settings.network_settings(cx);
        let network = (network != self.saved_network).then_some(network);
        let history = settings.search_history();
        let history = (history != self.saved_history).then_some(history);
        let highlight =
            (self.last_highlight != self.highlight_baseline).then(|| self.last_highlight.clone());
        self.save_pending = false;
        if draft.is_none() && network.is_none() && history.is_none() && highlight.is_none() {
            self.save_error = None;
            if self.closing {
                self.finish_close(window, cx);
            }
            cx.notify();
            return;
        }
        let Some(store) = self.workspace.read(cx).persistence.store.clone() else {
            self.save_error = Some(
                crate::tr!(
                    "状态库尚未就绪，修改尚未保存。请重试。",
                    "Storage is not ready. Changes have not been saved. Try again."
                )
                .to_owned(),
            );
            self.closing = false;
            cx.notify();
            return;
        };
        self.sync_retained_workspace(cx);
        self.saving = true;
        self.save_error = None;
        let tasks = self.workspace.update(cx, |owner, cx| {
            owner.settings_save_failed = false;
            if let Some(draft) = &draft {
                let next = merge_changes(&self.last_draft, draft, &owner.app_settings);
                owner.save_app_settings(next, window, cx);
            }
            let cloud_task = if let Some(network) = &network {
                owner.save_cloud_settings(network.clone(), window, cx);
                owner.persistence.state_tasks.pop()
            } else {
                None
            };
            let history_task = if let Some(history) = &history {
                let removed = self
                    .saved_history
                    .iter()
                    .filter(|query| !history.contains(query))
                    .cloned()
                    .collect::<Vec<_>>();
                owner.remove_search_history_entries(&removed, window, cx);
                owner.persistence.search_history_save_task.take()
            } else {
                None
            };
            if let Some(highlight) = &highlight {
                owner.autosave_highlight_settings(
                    highlight.clone(),
                    self.highlight_baseline.clone(),
                    window,
                    cx,
                );
            }
            [
                owner.persistence.app_settings_save_task.take(),
                cloud_task,
                history_task,
            ]
        });
        self.save_task = Some(cx.spawn_in(window, async move |this, cx| {
            for task in tasks.into_iter().flatten() {
                task.await;
            }
            let failed = this
                .update_in(cx, |this, _, cx| {
                    this.workspace.read(cx).settings_save_failed
                })
                .unwrap_or(true);
            // A batch can partly succeed. Reconcile each acknowledged section before retrying,
            // so reverting a successfully written field still persists that reversion.
            let persisted = if failed {
                cx.background_spawn(async move {
                    Ok::<_, anyhow::Error>((
                        store.load_app_settings()?,
                        store.load_cloud_settings()?,
                        store.load_search_history()?,
                        store.load_color_labels()?,
                    ))
                })
                .await
                .ok()
            } else {
                None
            };
            _ = this.update_in(cx, |this, window, cx| {
                this.saving = false;
                if let Some(draft) = draft.filter(|draft| {
                    !failed
                        || persisted.as_ref().is_some_and(|(saved, _, _, _)| {
                            merge_changes(&this.saved_draft, draft, saved) == *saved
                        })
                }) {
                    this.saved_draft = draft;
                }
                if let Some(network) = network.filter(|network| {
                    !failed
                        || persisted
                            .as_ref()
                            .is_some_and(|(_, saved, _, _)| network == saved)
                }) {
                    this.saved_network = network;
                }
                if let Some(history) = history.filter(|history| {
                    !failed
                        || persisted.as_ref().is_some_and(|(_, _, saved, _)| {
                            this.saved_history
                                .iter()
                                .filter(|query| !history.contains(query))
                                .all(|query| !saved.contains(query))
                        })
                }) {
                    this.saved_history = history;
                }
                if let Some(highlight) = highlight.filter(|draft| {
                    !failed
                        || persisted.as_ref().is_some_and(|(saved, _, _, labels)| {
                            let saved = crate::color_labels_dialog::LogColoringConfig {
                                highlight_log_levels: saved.highlight_log_levels,
                                log_coloring: saved.log_coloring.clone(),
                                selection_styles: saved.selection_styles.clone(),
                                keyword_match_styles: saved.keyword_match_styles.clone(),
                                labels: labels.clone(),
                            };
                            highlight_preferences::merge_highlight_changes(
                                draft,
                                &this.highlight_baseline,
                                saved.clone(),
                            ) == saved
                        })
                }) {
                    this.highlight_baseline = highlight;
                }
                if failed {
                    this.save_error = Some(
                        crate::tr!(
                            "修改未能全部保存，请重试。",
                            "Some changes could not be saved. Try again."
                        )
                        .to_owned(),
                    );
                    this.closing = false;
                } else {
                    if !this.quitting && (this.save_pending || this.closing) {
                        this.save(window, cx);
                    }
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

    fn request_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.saving && self.workspace.read(cx).settings_saving {
            self.closing = true;
            return;
        }
        self.draft_changed(window, cx);
        self.closing = true;
        self.save(window, cx);
    }

    fn finish_close(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.persist_size(window, cx);
        cx.global_mut::<WorkspaceWindowRegistry>().settings_window = None;
        window.remove_window();
    }

    pub(super) fn take_quit_save(&mut self, cx: &mut Context<Self>) -> SettingsQuitSave {
        self.quitting = true;
        self.debounce_task = None;
        self.sync_retained_workspace(cx);
        let editor = self.settings.read(cx);
        let owner = self.workspace.read(cx);
        let (draft, _) = editor.settings_for_autosave(&self.last_draft, cx);
        let settings = merge_changes(&self.last_draft, &draft, &owner.app_settings);
        let highlight = (self.last_highlight != self.highlight_baseline).then(|| {
            highlight_preferences::merge_highlight_changes(
                &self.last_highlight,
                &self.highlight_baseline,
                owner.highlight_config(),
            )
        });
        let network = editor.network_settings(cx);
        let history = editor.search_history();
        let removed = self
            .saved_history
            .iter()
            .filter(|query| !history.contains(query))
            .collect::<Vec<_>>();
        let history = (!removed.is_empty()).then(|| {
            owner
                .search_history
                .iter()
                .filter(|query| !removed.contains(query))
                .cloned()
                .collect()
        });
        SettingsQuitSave {
            store: owner.persistence.store.clone(),
            settings,
            highlight,
            network: (network != self.saved_network).then_some(network),
            history,
            previous: self.save_task.take(),
            _workspace: self.workspace.clone(),
        }
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

    pub(super) fn reset_finished(window: &mut Window, cx: &mut App) -> bool {
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
                _ = view.update(cx, |this, cx| {
                    let closing = this.closing;
                    *this = Self::new(this.workspace.clone(), None, window, cx);
                    if closing {
                        this.request_close(window, cx);
                    }
                    cx.notify();
                });
            });
        });
        true
    }
}

/// An immutable final batch also covers edits still inside the debounce interval.
/// It is flushed after existing workspace writes, without starting new foreground tasks.
pub(super) struct SettingsQuitSave {
    store: Option<Arc<StateStore>>,
    settings: AppSettings,
    highlight: Option<crate::color_labels_dialog::LogColoringConfig>,
    network: Option<CloudSettings>,
    history: Option<Vec<String>>,
    previous: Option<Task<()>>,
    _workspace: Entity<Workspace>,
}

impl SettingsQuitSave {
    pub(super) async fn flush(self, executor: gpui_kit::BackgroundExecutor) {
        if let Some(previous) = self.previous {
            previous.await;
        }
        let result = executor
            .spawn(async move {
                let store = self
                    .store
                    .ok_or_else(|| anyhow::anyhow!("Settings storage is not ready"))?;
                let mut settings = self.settings;
                store.save_app_settings(settings.clone())?;
                if let Some(highlight) = self.highlight {
                    settings.highlight_log_levels = highlight.highlight_log_levels;
                    settings.log_coloring = highlight.log_coloring;
                    settings.selection_styles = highlight.selection_styles;
                    settings.keyword_match_styles = highlight.keyword_match_styles;
                    store.save_highlight_settings(settings, &highlight.labels)?;
                }
                if let Some(network) = self.network {
                    store.save_cloud_settings(&network)?;
                }
                if let Some(history) = self.history {
                    store.save_search_history(&history)?;
                }
                Ok::<_, anyhow::Error>(())
            })
            .await;
        if let Err(error) = result {
            log::error!("Could not flush settings on quit: {error:#}");
        }
    }
}

impl Render for SettingsWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        window.set_window_title(crate::tr!("设置", "Settings"));
        let busy = self.busy(cx);
        let resetting = !self.saving && self.workspace.read(cx).settings_saving;
        v_flex()
            .id("settings-window-content")
            .size_full()
            .min_h_0()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .key_context(SETTINGS_CONTEXT)
            .track_focus(&self.focus)
            .when(resetting, |surface| {
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
                this.request_close(window, cx);
            }))
            .on_action(cx.listener(|this, _: &OpenSettings, window, cx| {
                this.request_close(window, cx);
            }))
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
                        this.request_close(window, cx);
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
                                let current = this.workspace.read(cx).app_settings.clone();
                                this.workspace.update(cx, |owner, cx| {
                                    owner.confirm_reset_app_settings(current, window, cx)
                                });
                            })),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .min_w_0()
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(
                                        if self.save_error.is_some()
                                            || self.validation_error.is_some()
                                        {
                                            cx.theme().danger
                                        } else {
                                            cx.theme().muted_foreground
                                        },
                                    )
                                    .child(
                                        self.save_error
                                            .clone()
                                            .or_else(|| self.validation_error.clone())
                                            .unwrap_or_else(|| {
                                                if self.saving || self.debounce_task.is_some() {
                                                    crate::tr!("正在自动保存…", "Saving changes…")
                                                        .to_owned()
                                                } else {
                                                    crate::tr!(
                                                        "更改会自动保存",
                                                        "Changes are saved automatically"
                                                    )
                                                    .to_owned()
                                                }
                                            }),
                                    ),
                            )
                            .when(self.save_error.is_some(), |row| {
                                row.child(
                                    Button::new("settings-retry-save")
                                        .label(crate::tr!("重试", "Retry"))
                                        .disabled(busy)
                                        .on_click(
                                            cx.listener(|this, _, window, cx| {
                                                this.save(window, cx)
                                            }),
                                        ),
                                )
                            }),
                    ),
            )
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
mod tests;
