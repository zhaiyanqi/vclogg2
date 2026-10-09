//! Own unfinished foreground work without retaining completed task allocations.
use std::{cell::RefCell, collections::BTreeMap, rc::Rc};

use gpui_kit::{App, Task};

#[derive(Default)]
pub(crate) struct PendingTasks {
    next_id: u64,
    tasks: Rc<RefCell<BTreeMap<u64, Task<()>>>>,
}

impl PendingTasks {
    pub(crate) fn push(&mut self, task: Task<()>, cx: &mut App) {
        let id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .expect("task identity exhausted");
        let tasks = Rc::downgrade(&self.tasks);
        let task = cx.spawn(async move |_| {
            task.await;
            if let Some(tasks) = tasks.upgrade() {
                // Release the borrow before dropping the currently completing task.
                let completed = tasks.borrow_mut().remove(&id);
                drop(completed);
            }
        });
        self.tasks.borrow_mut().insert(id, task);
    }

    pub(crate) fn pop(&mut self) -> Option<Task<()>> {
        self.tasks.borrow_mut().pop_last().map(|(_, task)| task)
    }

    /// Transfer ownership to a shutdown/save barrier without cancelling the work.
    pub(crate) fn take_all(&mut self) -> Vec<Task<()>> {
        std::mem::take(&mut *self.tasks.borrow_mut())
            .into_values()
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn clear(&mut self) {
        drop(self.take_all());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::TestAppContext;

    #[gpui_kit::test]
    fn completed_tasks_release_storage_without_cancelling_pending_work(cx: &mut TestAppContext) {
        let mut tasks = PendingTasks::default();
        let (send, receive) = async_channel::bounded::<()>(1);
        let completed = Rc::new(std::cell::Cell::new(false));
        cx.update(|cx| {
            let completed = completed.clone();
            let task = cx.spawn(async move |_| {
                receive.recv().await.unwrap();
                completed.set(true);
            });
            tasks.push(task, cx);
        });
        for _ in 0..100 {
            cx.update(|cx| tasks.push(Task::ready(()), cx));
            cx.run_until_parked();
            assert_eq!(tasks.tasks.borrow().len(), 1);
        }
        assert!(!completed.get());
        send.try_send(()).unwrap();
        cx.run_until_parked();
        assert!(completed.get());
        assert!(tasks.tasks.borrow().is_empty());
    }

    #[gpui_kit::test]
    async fn drained_and_popped_tasks_finish_after_the_owner_is_dropped(cx: &mut TestAppContext) {
        let mut tasks = PendingTasks::default();
        let (send, receive) = async_channel::bounded::<()>(2);
        let completed = Rc::new(std::cell::Cell::new(0));
        for _ in 0..2 {
            cx.update(|cx| {
                let completed = completed.clone();
                let receive = receive.clone();
                let task = cx.spawn(async move |_| {
                    receive.recv().await.unwrap();
                    completed.set(completed.get() + 1);
                });
                tasks.push(task, cx);
            });
        }
        cx.run_until_parked();
        let popped = tasks.pop().unwrap();
        let drained = tasks.take_all();
        drop(tasks);
        send.try_send(()).unwrap();
        send.try_send(()).unwrap();
        popped.await;
        for task in drained {
            task.await;
        }
        assert_eq!(completed.get(), 2);
    }

    #[gpui_kit::test]
    fn dropping_owner_cancels_work_and_releases_captures(cx: &mut TestAppContext) {
        let mut tasks = PendingTasks::default();
        let capture = Rc::new(());
        let weak_capture = Rc::downgrade(&capture);
        let (_send, receive) = async_channel::bounded::<()>(1);
        cx.update(|cx| {
            let task = cx.spawn(async move |_| {
                let _capture = capture;
                _ = receive.recv().await;
            });
            tasks.push(task, cx);
        });
        cx.run_until_parked();
        assert!(weak_capture.upgrade().is_some());
        drop(tasks);
        cx.run_until_parked();
        assert!(weak_capture.upgrade().is_none());
    }
}
