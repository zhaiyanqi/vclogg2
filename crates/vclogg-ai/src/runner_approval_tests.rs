use super::*;
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

#[derive(Default)]
struct Grants(Mutex<BTreeSet<(PathBuf, String)>>);
impl CommandApprovalStore for Grants {
    fn is_allowed(&self, directory: &Path, command: &str) -> anyhow::Result<bool> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .contains(&(directory.to_owned(), command.to_owned())))
    }
    fn allow(&self, directory: &Path, command: &str) -> anyhow::Result<()> {
        self.0
            .lock()
            .unwrap()
            .insert((directory.to_owned(), command.to_owned()));
        Ok(())
    }
}

async fn attempt(
    store: Arc<Grants>,
    root: &Path,
    command: &str,
    option: &str,
) -> (ToolResult, usize) {
    let (events, receiver) = async_channel::unbounded();
    let (sender, results) = async_channel::unbounded();
    sender
        .send(("call".into(), ToolResult::ok(json!({"option_id":option}))))
        .await
        .unwrap();
    let call = ToolCall {
        id: "call".into(),
        name: "shell".into(),
        arguments: json!({"command":command}),
    };
    let result = execute_shell_tool(
        &[],
        &Some(store),
        root,
        &call,
        &Cancellation::default(),
        &events,
        &results,
    )
    .await
    .unwrap();
    let mut questions = 0;
    while let Ok(event) = receiver.try_recv() {
        if let AgentEvent::QuestionRequested(question) = event {
            assert!(question.options.iter().any(|o| o.id == "allow_always"));
            questions += 1;
        }
    }
    (result, questions)
}

#[tokio::test]
async fn remember_matches_exact_directory_and_command_and_revoke_is_immediate() {
    let store = Arc::new(Grants::default());
    let dir = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let command = "echo approved";
    let (result, asked) = attempt(store.clone(), dir.path(), command, "allow_always").await;
    assert!(!result.is_error);
    assert_eq!(asked, 1);
    let (result, asked) = attempt(store.clone(), dir.path(), command, "deny").await;
    assert!(!result.is_error);
    assert_ne!(result.value["status"], "denied");
    assert_eq!(asked, 0);
    for (root, changed) in [
        (other.path(), command),
        (dir.path(), "echo different"),
        (dir.path(), "echo approved && echo extra"),
    ] {
        let (result, asked) = attempt(store.clone(), root, changed, "deny").await;
        assert_eq!(result.value["status"], "denied");
        assert_eq!(asked, 1);
    }
    store.0.lock().unwrap().clear();
    assert_eq!(
        attempt(store.clone(), dir.path(), command, "allow").await.1,
        1
    );
    assert_eq!(attempt(store, dir.path(), command, "deny").await.1, 1);
}

#[tokio::test]
async fn remembered_command_cannot_override_explicit_denial() {
    let store = Arc::new(Grants::default());
    let dir = tempfile::tempdir().unwrap();
    store
        .allow(&dir.path().canonicalize().unwrap(), "git reset --hard")
        .unwrap();
    let (result, questions) = attempt(store, dir.path(), "git reset --hard", "allow_always").await;
    assert!(result.is_error);
    assert_eq!(questions, 0);
}

struct UnwritableGrants;
impl CommandApprovalStore for UnwritableGrants {
    fn is_allowed(&self, _: &Path, _: &str) -> anyhow::Result<bool> {
        Ok(false)
    }
    fn allow(&self, _: &Path, _: &str) -> anyhow::Result<()> {
        anyhow::bail!("Cannot save grant")
    }
}
#[tokio::test]
async fn saving_failure_does_not_execute_or_report_a_saved_grant() {
    let root = tempfile::tempdir().unwrap();
    let (events, receiver) = async_channel::unbounded();
    let (sender, results) = async_channel::unbounded();
    sender
        .send((
            "one".into(),
            ToolResult::ok(json!({"option_id":"allow_always"})),
        ))
        .await
        .unwrap();
    let call = ToolCall {
        id: "one".into(),
        name: "shell".into(),
        arguments: json!({"command":"echo approved"}),
    };
    let result = execute_shell_tool(
        &[],
        &Some(Arc::new(UnwritableGrants)),
        root.path(),
        &call,
        &Cancellation::default(),
        &events,
        &results,
    )
    .await;
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Cannot save grant")
    );
    while let Ok(event) = receiver.try_recv() {
        assert!(!matches!(event, AgentEvent::ExtensionToolStarted(_)));
    }
}
