//! A submitted follow-up keeps its model and source choices even if the draft changes.
use super::*;
use vclogg_ai::AiSettings;

#[derive(Clone)]
pub(super) struct RunPreferences {
    settings: AiSettings,
    provider_id: Option<String>,
    selected_log_ids: Option<BTreeSet<u64>>,
    selected_project_directories: Option<BTreeSet<PathBuf>>,
    include_search_directory: bool,
}
impl RunPreferences {
    pub(super) fn capture(session: &ConversationSession) -> Self {
        Self {
            settings: session.settings.clone(),
            provider_id: session.conversation.provider_id.clone(),
            selected_log_ids: session.selected_log_ids.clone(),
            selected_project_directories: session.selected_project_directories.clone(),
            include_search_directory: session.include_search_directory,
        }
    }
    pub(super) fn model(&self) -> &str {
        self.settings
            .providers
            .iter()
            .find(|p| Some(&p.id) == self.provider_id.as_ref())
            .map_or("", |p| p.model.as_str())
    }
    // send() captures all execution inputs synchronously before spawning the run.
    // Swap back immediately afterwards so the next draft retains the user's choices.
    pub(super) fn swap(&mut self, session: &mut ConversationSession) {
        std::mem::swap(&mut self.settings, &mut session.settings);
        std::mem::swap(&mut self.provider_id, &mut session.conversation.provider_id);
        std::mem::swap(&mut self.selected_log_ids, &mut session.selected_log_ids);
        std::mem::swap(
            &mut self.selected_project_directories,
            &mut session.selected_project_directories,
        );
        std::mem::swap(
            &mut self.include_search_directory,
            &mut session.include_search_directory,
        );
    }
}
