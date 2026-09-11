use super::*;

impl PredefinedFiltersDialog {
    fn cloud_detail_update(&self, detail: &CloudFilterItem, cx: &gpui::App) -> CloudFilterUpdate {
        CloudFilterUpdate {
            name: self.cloud_detail_name.read(cx).value().trim().to_string(),
            value: self.cloud_detail_value.read(cx).value().trim().to_string(),
            use_regex: self.cloud_detail_use_regex,
            note: self.cloud_detail_note.read(cx).value().trim().to_string(),
            collaborative: detail.can_delete.then_some(self.cloud_detail_collaborative),
            base_revision: Some(detail.revision),
        }
    }

    pub(super) fn cloud_detail_has_changes(
        &self,
        detail: &CloudFilterItem,
        cx: &gpui::App,
    ) -> bool {
        let update = self.cloud_detail_update(detail, cx);
        update.name != detail.name
            || update.value != detail.value
            || update.use_regex != detail.use_regex
            || update.note != detail.note
            || update
                .collaborative
                .is_some_and(|value| value != detail.collaborative)
    }

    pub(super) fn save_cloud_detail(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.cloud_task.is_some()
            || self.cloud_offline
            || !self.cloud_connection.as_ref().is_some_and(|connection| {
                connection.connected && connection.supports_uuid_filter_branches()
            })
        {
            return;
        }
        let Some(detail) = self.cloud_detail.clone().filter(|detail| detail.can_edit) else {
            return;
        };
        let Some(client) = self.cloud_client.clone() else {
            return;
        };
        let update = self.cloud_detail_update(&detail, cx);
        if update.name.is_empty() || update.value.is_empty() {
            self.cloud_message = Some(
                crate::tr!(
                    "名称和匹配值不能为空",
                    "Name and match value can’t be empty"
                )
                .to_string(),
            );
            cx.notify();
            return;
        }
        if !self.cloud_detail_has_changes(&detail, cx) {
            return;
        }
        self.cloud_message = None;
        self.cloud_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = cx.background_spawn(async move {
                let mutation = client.update_filter(&detail.id, &update)?;
                // A failed follow-up read must not turn a committed write into a save failure.
                let mut saved = detail;
                saved.name = update.name;
                saved.value = update.value;
                saved.use_regex = update.use_regex;
                saved.note = mutation.note;
                saved.collaborative = mutation.collaborative;
                saved.revision = mutation.revision;
                saved.updated_at = chrono::Utc::now().timestamp_millis();
                let refreshed = client.get_filter(&saved.id);
                let revisions = client.list_revisions(&saved.id, 1, 30);
                Ok::<_, anyhow::Error>((refreshed.unwrap_or(saved), revisions))
            }).await;
            _ = this.update_in(cx, |this, window, cx| {
                this.cloud_task = None;
                match result {
                    Ok((saved, revisions)) => {
                        if let Some(item) = this.cloud_items.iter_mut().find(|item| item.id == saved.id) {
                            *item = saved.clone();
                        }
                        if this.cloud_detail.as_ref().is_some_and(|detail| detail.id == saved.id) {
                            this.set_cloud_detail_draft(&saved, window, cx);
                            this.cloud_detail = Some(saved);
                            this.cloud_revision = None;
                            match revisions {
                                Ok(page) => {
                                    this.cloud_revisions = page.items;
                                    this.cloud_revision_page = page.page;
                                    this.cloud_revision_page_size = page.page_size;
                                    this.cloud_revision_total = page.total;
                                    this.cloud_message = Some(crate::tr!(
                                        "云端修改已保存", "Cloud changes saved"
                                    ).to_string());
                                }
                                Err(error) => {
                                    this.cloud_message = Some(crate::tr_args!(
                                        "云端修改已保存，但刷新编辑记录失败：{error}",
                                        "Cloud changes saved, but revision history couldn’t be refreshed: {error}"
                                    ));
                                }
                            }
                        }
                        // Remote edits leave local drafts and their merge baseline intact.
                        window.notify_message(crate::tr!("云端修改已保存", "Cloud changes saved"), cx);
                    }
                    Err(error) => {
                        this.cloud_message = Some(if cloud_error(&error).and_then(|error| error.code())
                            == Some("revision_conflict")
                        {
                            crate::tr!(
                                "云端版本已变化，当前编辑内容已保留且未提交。请复制需要保留的内容，关闭并重新打开详情后核对最新版本",
                                "The cloud version changed. Your draft is preserved and wasn’t submitted. Copy any changes you need, then close and reopen the details to review the latest version"
                            ).to_string()
                        } else {
                            crate::tr_args!("保存云端修改失败：{error}", "Couldn’t save cloud changes: {error}")
                        });
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
}
