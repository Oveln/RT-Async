use core::{
    cell::SyncUnsafeCell,
    pin::Pin,
    ptr::NonNull,
    sync::atomic::Ordering,
    task::{Context, Waker},
};

use portable_atomic::AtomicPtr;

use crate::{
    spawner::SpawnToken,
    task::{
        header::{TaskInfo, TaskRef},
        run_queue::RunQueueItem,
        state::State,
        waker,
    },
    util::UninitCell,
};

/// Error returned by [`TaskStorage::spawn`] when the task is already spawned.
#[derive(Debug)]
pub struct SpawnError;

/// JoinHandle state: holds the task output and the waker to wake on completion.
pub(crate) struct JoinState<T> {
    pub(crate) result: SyncUnsafeCell<Option<T>>,
    pub(crate) waker: SyncUnsafeCell<Option<Waker>>,
}

impl<T> JoinState<T> {
    pub const fn new() -> Self {
        Self {
            result: SyncUnsafeCell::new(None),
            waker: SyncUnsafeCell::new(None),
        }
    }
}

impl TaskRef {
    /// Returns a reference to the [`JoinState<T>`] field inside the
    /// [`TaskStorage<F>`] that this `TaskRef` points to.
    ///
    /// Relies on `#[repr(C)]` layout: `JoinState<T>` is the second field,
    /// at offset `size_of::<TaskInfo>()` rounded up to alignment of `JoinState<T>`.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `T` matches `F::Output` of the original
    /// `TaskStorage<F>` from which this `TaskRef` was created.
    pub(crate) unsafe fn join_state<T>(&self) -> &JoinState<T> {
        unsafe {
            let base = self.as_ptr() as *const u8;
            let offset = core::mem::size_of::<TaskInfo>()
                .next_multiple_of(core::mem::align_of::<JoinState<T>>());
            &*(base.add(offset) as *const JoinState<T>)
        }
    }

    /// Create a [`TaskRef`] pointing to the [`TaskInfo`] header inside a
    /// [`TaskStorage`].  Relies on `#[repr(C)]` layout — `TaskInfo` is the
    /// first field, so casting the `TaskStorage` pointer yields the correct
    /// `TaskInfo` pointer.
    pub(crate) fn new<F: Future + 'static>(task: &'static TaskStorage<F>) -> TaskRef {
        Self {
            ptr: NonNull::from(task).cast(),
        }
    }
}

/// Statically allocated future storage for a single task.
///
/// `#[repr(C)]` ensures [`TaskInfo`] is at offset 0, which allows
/// [`TaskRef::new`] to cast a `&TaskStorage<F>` pointer directly to a
/// `NonNull<TaskInfo>`.
#[repr(C)]
pub struct TaskStorage<F: Future + 'static> {
    pub(crate) info: TaskInfo,
    join: JoinState<F::Output>,
    f: UninitCell<F>,
}

impl<F: Future + 'static> TaskStorage<F> {
    pub const fn new() -> Self {
        Self {
            info: TaskInfo {
                state: State::new(),
                run_queue_item: RunQueueItem::new(),
                executor_ptr: AtomicPtr::new(core::ptr::null_mut()),
                poll_fn: SyncUnsafeCell::new(None),
            },
            join: JoinState::new(),
            f: UninitCell::uninit(),
        }
    }

    /// Poll the task's future.
    ///
    /// # Safety
    ///
    /// `p` must be a [`TaskRef`] obtained from the same `TaskStorage<F>`
    /// instance via [`TaskRef::new`].
    ///
    /// # Completion
    ///
    /// When the future returns `Ready`, this function:
    /// 1. Drops the future in place.
    /// 2. Stores `null` into `executor_ptr` with [`Release`] ordering so that
    ///    [`wake_task`] will skip this task on any subsequent wake.
    /// 3. Replaces `poll_fn` with `poll_exited` (a no-op) so that if the task
    ///    was re-enqueued before step 2, polling it again is harmless.
    ///
    /// The Release store of `executor_ptr` pairs with the Acquire load in
    /// [`wake_task`], guaranteeing visibility of the replaced poll function.
    ///
    /// [`Release`]: Ordering::Release
    /// [`wake_task`]: crate::executor::wake_task
    unsafe fn poll(p: TaskRef) {
        let this = unsafe { &*(p.as_ptr().cast::<TaskStorage<F>>()) };
        let future = unsafe { Pin::new_unchecked(this.f.as_mut()) };
        let waker = unsafe { waker::from_task(p) };
        let mut cx = Context::from_waker(&waker);

        // Replaced into poll_fn after the future completes — polling a
        // completed task is a no-op.
        unsafe fn poll_exited(_p: TaskRef) {}

        match future.poll(&mut cx) {
            core::task::Poll::Ready(output) => {
                unsafe {
                    *this.join.result.get() = Some(output);
                    this.info
                        .executor_ptr
                        .store(core::ptr::null_mut(), Ordering::Release);

                    this.f.drop_in_place();
                    *this.info.poll_fn.get() = Some(poll_exited);
                }
                this.info.state.despawn();
                unsafe {
                    if let Some(w) = (*this.join.waker.get()).take() {
                        w.wake();
                    }
                }
            }
            core::task::Poll::Pending => {}
        }
    }

    /// Spawn the task by initializing its future and poll function.
    ///
    /// Returns `Ok(SpawnToken)` on first call.  Returns `Err(SpawnError)` if
    /// the task has already been spawned (idempotency check via the atomic
    /// `STATE_SPAWNED` flag).
    ///
    /// The returned [`SpawnToken`] must be consumed by [`Spawner::spawn`] —
    /// dropping it will panic.
    ///
    /// [`Spawner::spawn`]: crate::spawner::Spawner::spawn
    pub fn spawn(&'static self, f: impl FnOnce() -> F) -> Result<SpawnToken<F>, SpawnError> {
        if !self.info.state.spawn() {
            return Err(SpawnError);
        }
        unsafe {
            *self.join.result.get() = None;
            *self.join.waker.get() = None;
            *self.info.poll_fn.get() = Some(TaskStorage::<F>::poll);
            self.f.write_in_place(f);
        }
        let task = TaskRef::new(self);
        Ok(SpawnToken::new(task))
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use std::format;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::task::{Context, Poll};

    use super::*;

    #[test]
    fn spawn_returns_ok() {
        static TASK: TaskStorage<std::future::Ready<()>> = TaskStorage::new();
        let result = TASK.spawn(|| std::future::ready(()));
        assert!(result.is_ok());
        let token = result.unwrap();
        core::mem::forget(token);
    }

    #[test]
    fn double_spawn_returns_err() {
        static TASK: TaskStorage<std::future::Ready<()>> = TaskStorage::new();
        let token1 = TASK.spawn(|| std::future::ready(())).unwrap();
        core::mem::forget(token1);
        let result = TASK.spawn(|| std::future::ready(()));
        assert!(result.is_err());
    }

    #[test]
    fn respawn_after_poll_completion() {
        static TASK: TaskStorage<std::future::Ready<()>> = TaskStorage::new();

        let token = TASK.spawn(|| std::future::ready(())).unwrap();
        let task = token.task_ref;
        core::mem::forget(token);
        unsafe {
            let poll = (*task.info().poll_fn.get()).unwrap();
            poll(task);
        }
        let result = TASK.spawn(|| std::future::ready(()));
        assert!(result.is_ok(), "Should be able to respawn after completion");
        core::mem::forget(result.unwrap());
    }

    #[test]
    fn cannot_respawn_while_running() {
        static TASK: TaskStorage<std::future::Ready<()>> = TaskStorage::new();
        let token = TASK.spawn(|| std::future::ready(())).unwrap();
        core::mem::forget(token);
        let result = TASK.spawn(|| std::future::ready(()));
        assert!(result.is_err());
    }

    #[test]
    fn task_storage_new_is_unspawned() {
        static TASK: TaskStorage<std::future::Ready<()>> = TaskStorage::new();
        let token = TASK.spawn(|| std::future::ready(())).unwrap();
        core::mem::forget(token);
    }

    #[test]
    fn spawn_error_debug() {
        let err = SpawnError;
        let msg = format!("{:?}", err);
        assert!(!msg.is_empty());
    }

    #[test]
    fn future_is_polled_correctly() {
        static POLLED: AtomicUsize = AtomicUsize::new(0);
        static TASK: TaskStorage<PollCounter> = TaskStorage::new();

        struct PollCounter;
        impl std::future::Future for PollCounter {
            type Output = ();
            fn poll(self: std::pin::Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
                POLLED.fetch_add(1, Ordering::Relaxed);
                Poll::Ready(())
            }
        }

        let token = TASK.spawn(|| PollCounter).unwrap();
        let task = token.task_ref;
        core::mem::forget(token);
        unsafe {
            let poll = (*task.info().poll_fn.get()).unwrap();
            poll(task);
        }
        assert_eq!(POLLED.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn pending_future_wake_without_executor() {
        static POLLS: AtomicUsize = AtomicUsize::new(0);
        static TASK: TaskStorage<PendingOnce> = TaskStorage::new();

        struct PendingOnce;
        impl std::future::Future for PendingOnce {
            type Output = ();
            fn poll(self: std::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
                let n = POLLS.fetch_add(1, Ordering::Relaxed);
                if n == 0 {
                    cx.waker().wake_by_ref();
                    Poll::Pending
                } else {
                    Poll::Ready(())
                }
            }
        }

        let token = TASK.spawn(|| PendingOnce).unwrap();
        let task = token.task_ref;
        core::mem::forget(token);
        unsafe {
            let poll = (*task.info().poll_fn.get()).unwrap();
            poll(task);
        }
        // Without an executor, wake_by_ref is a no-op (executor_ptr is null),
        // so the future is polled exactly once and remains Pending.
        assert_eq!(POLLS.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn poll_fn_is_exited_after_ready() {
        static TASK: TaskStorage<std::future::Ready<()>> = TaskStorage::new();
        let token = TASK.spawn(|| std::future::ready(())).unwrap();
        let task = token.task_ref;
        core::mem::forget(token);
        unsafe {
            let poll = (*task.info().poll_fn.get()).unwrap();
            poll(task);
            let poll2 = (*task.info().poll_fn.get()).unwrap();
            poll2(task);
        }
    }

    #[test]
    fn multiple_spawn_cycles() {
        static COUNT: AtomicUsize = AtomicUsize::new(0);
        static TASK: TaskStorage<CountOnce> = TaskStorage::new();

        struct CountOnce;
        impl std::future::Future for CountOnce {
            type Output = ();
            fn poll(self: std::pin::Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
                COUNT.fetch_add(1, Ordering::Relaxed);
                Poll::Ready(())
            }
        }

        for _ in 0..5 {
            let token = TASK.spawn(|| CountOnce).unwrap();
            let task = token.task_ref;
            core::mem::forget(token);
            unsafe {
                let poll = (*task.info().poll_fn.get()).unwrap();
                poll(task);
            }
        }
        assert_eq!(COUNT.load(Ordering::Relaxed), 5);
    }

    #[test]
    fn task_ref_new_casts_correctly() {
        static TASK: TaskStorage<std::future::Ready<()>> = TaskStorage::new();
        let token = TASK.spawn(|| std::future::ready(())).unwrap();
        let taskref = token.task_ref;
        core::mem::forget(token);

        let info_ptr = taskref.as_ptr();
        let storage_ptr = core::ptr::from_ref(&TASK) as *const ();
        assert_eq!(
            info_ptr as *const (), storage_ptr,
            "TaskRef should point to the start of TaskStorage (TaskInfo is at offset 0)"
        );
    }
}
