//! Native directory notifications with bounded pending work and a slow safety check.
//! No GPUI entities or log contents are accessed by the native callback.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};

use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher as _};

use crate::path_identity::{PathMatchKey, normalized_path_match_key};

pub(crate) const RECHECK_INTERVAL: Duration = Duration::from_secs(30);
pub(crate) const REFRESH_INTERVAL: Duration = Duration::from_millis(400);
const RETRY_INTERVAL: Duration = Duration::from_secs(5);

type Targets = BTreeMap<u64, PathBuf>;

#[derive(Default)]
struct WatchState {
    targets: Targets,
    aliases: BTreeMap<u64, BTreeSet<PathMatchKey>>,
    dirty: BTreeSet<u64>,
    retry: BTreeSet<u64>,
    paused: bool,
    repair: bool,
    stopped: bool,
}

/// Owned by a foreground document scope; dropping it stops the worker and releases watches.
pub(crate) struct FileWatch {
    state: Arc<Mutex<WatchState>>,
    control: mpsc::SyncSender<()>,
    wake: async_channel::Sender<()>,
    receiver: async_channel::Receiver<()>,
}

impl FileWatch {
    pub(crate) fn new() -> std::io::Result<Self> {
        let state = Arc::new(Mutex::new(WatchState::default()));
        let (control, commands) = mpsc::sync_channel(1);
        let (wake, receiver) = async_channel::bounded(1);
        let worker_state = state.clone();
        let worker_wake = wake.clone();
        let worker_control = control.clone();
        std::thread::Builder::new()
            .name("log-file-watch".into())
            .spawn(move || run_worker(worker_state, worker_wake, worker_control, commands))?;
        Ok(Self {
            state,
            control,
            wake,
            receiver,
        })
    }

    pub(crate) fn receiver(&self) -> async_channel::Receiver<()> {
        self.receiver.clone()
    }

    /// In-memory lifecycle synchronization. Never notifies the observed GPUI entity.
    pub(crate) fn sync(&self, targets: Targets, paused: bool) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let changed = state.targets != targets;
        if changed {
            state.dirty.retain(|id| targets.contains_key(id));
            state.retry.retain(|id| targets.contains_key(id));
            for (id, path) in &targets {
                if state.targets.get(id) != Some(path) {
                    state.dirty.insert(*id);
                }
            }
            state.targets = targets;
            _ = self.control.try_send(());
        }
        let resumed = state.paused && !paused;
        state.paused = paused;
        if (changed || resumed) && !paused && !state.dirty.is_empty() {
            _ = self.wake.try_send(());
        }
    }

    pub(crate) fn take_dirty(&self) -> BTreeSet<u64> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.paused {
            return BTreeSet::new();
        }
        std::mem::take(&mut state.dirty)
    }

    /// A failed/cancelled refresh gets one delayed recheck, even without another OS event.
    pub(crate) fn retry(&self, ids: impl IntoIterator<Item = u64>) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let mut added = false;
        for id in ids {
            if state.targets.contains_key(&id) {
                added |= state.retry.insert(id);
            }
        }
        if added {
            _ = self.control.try_send(());
        }
    }

    pub(crate) fn restore_dirty(&self, ids: impl IntoIterator<Item = u64>) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        for id in ids {
            if state.targets.contains_key(&id) {
                state.dirty.insert(id);
            }
        }
        if !state.paused && !state.dirty.is_empty() {
            _ = self.wake.try_send(());
        }
    }
}

impl Drop for FileWatch {
    fn drop(&mut self) {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .stopped = true;
        _ = self.control.try_send(());
        self.receiver.close();
        // Do not join a platform watcher thread on the UI thread.
    }
}

fn signal(state: &WatchState, wake: &async_channel::Sender<()>) {
    if !state.stopped && !state.paused && !state.dirty.is_empty() {
        _ = wake.try_send(());
    }
}

fn receive_event(
    event: notify::Result<Event>,
    state: &Mutex<WatchState>,
    wake: &async_channel::Sender<()>,
    control: &mpsc::SyncSender<()>,
) {
    let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
    if state.stopped {
        return;
    }
    match event {
        Ok(event) if event.need_rescan() || event.paths.is_empty() => {
            state.dirty = state.targets.keys().copied().collect();
            state.repair = true;
        }
        Ok(event) => {
            // Reads performed by the viewer must not feed back into refresh notifications.
            if event.kind.is_access() {
                return;
            }
            let paths = event
                .paths
                .iter()
                .map(|path| normalized_path_match_key(path))
                .collect::<BTreeSet<_>>();
            let affected = state
                .aliases
                .iter()
                .filter(|(id, _)| state.targets.contains_key(*id))
                .filter_map(|(id, aliases)| {
                    aliases
                        .iter()
                        .any(|alias| paths.contains(alias))
                        .then_some(*id)
                })
                .collect::<Vec<_>>();
            if affected.is_empty() {
                return;
            }
            state.dirty.extend(affected);
            if event.kind.is_remove()
                || matches!(
                    event.kind,
                    notify::EventKind::Modify(notify::event::ModifyKind::Name(_))
                )
            {
                // The parent directory itself can have moved. Repair on the worker thread.
                state.repair = true;
            }
        }
        Err(error) => {
            log::warn!("File notification failed; falling back to reconciliation: {error}");
            state.dirty = state.targets.keys().copied().collect();
            state.repair = true;
        }
    }
    signal(&state, wake);
    if state.repair {
        _ = control.try_send(());
    }
}

fn make_watcher(
    state: &Arc<Mutex<WatchState>>,
    wake: &async_channel::Sender<()>,
    control: &mpsc::SyncSender<()>,
) -> Option<RecommendedWatcher> {
    let state = state.clone();
    let wake = wake.clone();
    let control = control.clone();
    match notify::recommended_watcher(move |event| receive_event(event, &state, &wake, &control)) {
        Ok(watcher) => Some(watcher),
        Err(error) => {
            log::warn!("Native file watching unavailable; using fallback checks: {error}");
            None
        }
    }
}

fn target_paths(path: &Path) -> Vec<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(path)
    };
    let mut paths = vec![absolute.clone()];
    // Keep both the named path and its target so symlink replacement and target writes work.
    if let Ok(canonical) = absolute.canonicalize() {
        paths.push(canonical);
    } else if let Some(parent) = absolute.parent()
        && let (Ok(parent), Some(name)) = (parent.canonicalize(), absolute.file_name())
    {
        paths.push(parent.join(name));
    }
    paths
}

fn run_worker(
    shared: Arc<Mutex<WatchState>>,
    wake: async_channel::Sender<()>,
    control: mpsc::SyncSender<()>,
    commands: mpsc::Receiver<()>,
) {
    let mut watcher = make_watcher(&shared, &wake, &control);
    let mut watched = BTreeSet::<PathBuf>::new();
    let mut installed = Targets::new();
    let mut fallback = BTreeSet::new();
    let mut audit_due = Instant::now() + RECHECK_INTERVAL;
    let mut retry_due = None;
    let mut repair_pending = false;
    loop {
        let (targets, repair, stopped) = {
            let mut state = shared.lock().unwrap_or_else(|error| error.into_inner());
            (
                state.targets.clone(),
                std::mem::take(&mut state.repair),
                state.stopped,
            )
        };
        if stopped {
            break;
        }
        let now = Instant::now();
        repair_pending |= repair;
        let audit = now >= audit_due;
        let retry = retry_due.is_some_and(|due| now >= due);
        if targets != installed || audit || (retry && (repair_pending || !fallback.is_empty())) {
            if watcher.is_none() && !targets.is_empty() {
                watcher = make_watcher(&shared, &wake, &control);
            }
            let mut directories = BTreeMap::<PathBuf, BTreeSet<u64>>::new();
            let mut aliases = BTreeMap::<u64, BTreeSet<PathMatchKey>>::new();
            let mut recheck = targets
                .iter()
                .filter_map(|(id, path)| (installed.get(id) != Some(path)).then_some(*id))
                .collect::<BTreeSet<_>>();
            for (id, path) in &targets {
                for path in target_paths(path) {
                    let keys = aliases.entry(*id).or_default();
                    keys.insert(normalized_path_match_key(&path));
                    if let Some(parent) = path.parent() {
                        keys.insert(normalized_path_match_key(parent));
                        directories
                            .entry(parent.to_path_buf())
                            .or_default()
                            .insert(*id);
                    }
                }
            }
            // Install aliases before registration to close the initial notification race.
            shared
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .aliases = aliases;
            let remove = watched
                .iter()
                .filter(|path| repair_pending || audit || !directories.contains_key(*path))
                .cloned()
                .collect::<Vec<_>>();
            for path in remove {
                if let Some(watcher) = watcher.as_mut() {
                    _ = watcher.unwatch(&path);
                }
                watched.remove(&path);
            }
            fallback.clear();
            for (path, ids) in directories {
                if !watched.contains(&path) {
                    recheck.extend(ids.iter().copied());
                    if watcher.as_mut().is_some_and(|watcher| {
                        watcher.watch(&path, RecursiveMode::NonRecursive).is_ok()
                    }) {
                        watched.insert(path);
                    } else {
                        fallback.extend(ids);
                    }
                }
            }
            let mut state = shared.lock().unwrap_or_else(|error| error.into_inner());
            // Recheck after registration, including changes during the registration gap.
            // Retrying a failed directory must not poll unrelated healthy directories.
            let ids = recheck
                .iter()
                .filter(|id| state.targets.contains_key(*id))
                .copied()
                .collect::<Vec<_>>();
            state.dirty.extend(ids);
            signal(&state, &wake);
            installed = targets;
            repair_pending = false;
        }
        if audit || retry {
            let mut state = shared.lock().unwrap_or_else(|error| error.into_inner());
            if audit {
                let ids = state.targets.keys().copied().collect::<Vec<_>>();
                state.dirty.extend(ids);
                audit_due = now + RECHECK_INTERVAL;
            }
            if retry {
                let ids = std::mem::take(&mut state.retry);
                state.dirty.extend(ids);
                let ids = fallback
                    .iter()
                    .filter(|id| state.targets.contains_key(*id))
                    .copied()
                    .collect::<Vec<_>>();
                state.dirty.extend(ids);
                retry_due = None;
            }
            signal(&state, &wake);
        }
        let has_retry = !shared
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .retry
            .is_empty();
        // Bound repair attempts too: repeated rename/error events cannot cause a
        // tight unregister/register loop while a writer rotates or recreates files.
        if has_retry || !fallback.is_empty() || repair_pending {
            retry_due.get_or_insert(now + RETRY_INTERVAL);
        }
        let deadline = retry_due.map_or(audit_due, |due| due.min(audit_due));
        match commands.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(()) | Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
}
