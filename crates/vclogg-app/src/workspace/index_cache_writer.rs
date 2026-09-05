//! Best-effort index persistence outside directory search completion.
use std::sync::{
    OnceLock,
    mpsc::{self, SyncSender},
};

use vclogg_core::{PendingIndexCacheWrite, SearchCancellation};

type CacheJob = (PendingIndexCacheWrite, SearchCancellation);

pub(super) fn enqueue(write: PendingIndexCacheWrite, cancellation: SearchCancellation) {
    static WRITER: OnceLock<Option<SyncSender<CacheJob>>> = OnceLock::new();
    let sender = WRITER.get_or_init(|| {
        start_writer(2, |(write, cancellation): CacheJob| {
            if !cancellation.is_cancelled() {
                _ = write.persist();
            }
        })
        .ok()
    });
    if let Some(sender) = sender {
        // At most two pending indexes plus the active write. Cache writes are
        // optional: a full queue drops this cache, never blocks a search worker.
        _ = sender.try_send((write, cancellation));
    }
}

fn start_writer<T: Send + 'static>(
    capacity: usize,
    write: impl Fn(T) + Send + 'static,
) -> std::io::Result<SyncSender<T>> {
    let (sender, receiver) = mpsc::sync_channel(capacity);
    std::thread::Builder::new()
        .name("index-cache-writer".into())
        .spawn(move || {
            for job in receiver {
                write(job);
            }
        })?;
    Ok(sender)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn blocked_disk_write_does_not_block_enqueue_or_grow_the_queue() {
        let (started, receive_started) = mpsc::channel();
        let (release, receive_release) = mpsc::channel();
        let (completed, receive_completed) = mpsc::channel();
        let sender = start_writer(2, move |job| {
            started.send(job).unwrap();
            receive_release.recv().unwrap();
            completed.send(job).unwrap();
        })
        .unwrap();
        sender.try_send(0).unwrap();
        assert_eq!(
            receive_started
                .recv_timeout(Duration::from_secs(5))
                .unwrap(),
            0
        );
        sender.try_send(1).unwrap();
        sender.try_send(2).unwrap();
        assert!(matches!(
            sender.try_send(3),
            Err(mpsc::TrySendError::Full(3))
        ));
        for job in 0..3 {
            release.send(()).unwrap();
            assert_eq!(
                receive_completed
                    .recv_timeout(Duration::from_secs(5))
                    .unwrap(),
                job
            );
        }
    }
}
