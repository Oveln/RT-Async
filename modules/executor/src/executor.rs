use core::cell::UnsafeCell;
use core::sync::atomic::Ordering;

use crate::task::{TaskRef, run_queue::RunQueue};
use critical_section::with as locked;

/// Type-erased bitmap operations injected by [`Spawner::init`].
///
/// Each [`Executor`] stores an optional `BitmapOps` so it can signal the
/// scheduler's priority bitmap when tasks are enqueued (set) or drained
/// (clear).  The function pointers are monomorphised over the `Spawner`'s
/// const-generic group count, erasing that type from `Executor`.
///
/// Both `set` and `clear` receive a [`CriticalSection`] token so they can
/// access the bitmap through [`critical_section::Mutex::borrow`] without
/// triggering a nested critical section.
///
/// [`Spawner::init`]: crate::spawner::Spawner::init
pub(crate) struct BitmapOps {
    pub(crate) ptr: *mut (),
    pub(crate) set: unsafe fn(*mut (), usize, critical_section::CriticalSection),
    pub(crate) clear: unsafe fn(*mut (), usize, critical_section::CriticalSection),
}

/// Single-priority executor that drives ready tasks to completion.
///
/// Each priority level in the [`Spawner`] owns one `Executor`.  It maintains
/// a [`RunQueue`] of ready tasks and notifies the scheduler's priority bitmap
/// via the stored [`BitmapOps`] when the queue transitions between empty and
/// non-empty.
///
/// `BitmapOps` is installed once by [`Spawner::init`] and remains valid for
/// the lifetime of the pinned `Spawner`.
///
/// [`Spawner`]: crate::spawner::Spawner
/// [`Spawner::init`]: crate::spawner::Spawner::init
pub(crate) struct Executor {
    run_queue: RunQueue,
    priority: usize,
    bitmap_ops: UnsafeCell<BitmapOps>,
}

// SAFETY: Executor is shared across threads (ISR context). All interior
// mutability goes through UnsafeCell fields with the following invariants:
// - `bitmap_ops`: written once in `set_bitmap_ops` before any concurrent
//   access (called from `Spawner::init` which runs before spawning). After
//   init it is only read, so no data race.
// - `run_queue`: protected by `critical_section::Mutex`.
unsafe impl Sync for Executor {}

impl Executor {
    pub(crate) fn new(priority: usize) -> Self {
        Self {
            run_queue: RunQueue::new(),
            priority,
            bitmap_ops: UnsafeCell::new(BitmapOps {
                ptr: core::ptr::null_mut(),
                set: Self::not_initialized,
                clear: Self::not_initialized,
            }),
        }
    }

    unsafe fn not_initialized(_: *mut (), _: usize, _: critical_section::CriticalSection) {
        panic!("executor: bitmap ops not initialized, call Spawner::init first")
    }

    /// Install the bitmap operation callbacks.
    ///
    /// Called by [`Spawner::init`] to wire up the type-erased `set`/`clear`
    /// functions that update the scheduler's priority bitmap.
    ///
    /// [`Spawner::init`]: crate::spawner::Spawner::init
    pub(crate) fn set_bitmap_ops(&self, ops: BitmapOps) {
        // SAFETY: Called once from Spawner::init before any concurrent access.
        unsafe {
            *self.bitmap_ops.get() = ops;
        }
    }

    /// Drain all ready tasks from the run queue, polling each in turn.
    ///
    /// When the queue becomes empty, clears this executor's bit in the
    /// scheduler priority bitmap (via [`BitmapOps::clear`]) so that
    /// [`Spawner::try_preempt`] no longer returns this priority.
    ///
    /// [`Spawner::try_preempt`]: crate::spawner::Spawner::try_preempt
    pub(crate) fn run(&self) {
        log::trace!("executor[{}]: run start", self.priority);
        let ops = unsafe { &*self.bitmap_ops.get() };
        loop {
            let task = locked(|cs| {
                let task = self.run_queue.dequeue(cs);
                if let Some(task) = task {
                    Some(task)
                } else {
                    unsafe { (ops.clear)(ops.ptr, self.priority, cs) };
                    None
                }
            });
            if let Some(task) = task {
                log::trace!(
                    "executor[{}]: polling task {:p}",
                    self.priority,
                    task.info()
                );
                unsafe {
                    let poll = (*task.info().poll_fn.get()).unwrap();
                    poll(task)
                }
            } else {
                break;
            }
        }
        log::trace!("executor[{}]: run end", self.priority);
    }

    /// Enqueue a task and signal the scheduler bitmap.
    ///
    /// The task's `executor_ptr` back-pointer is stored so that future wakes
    /// route back to this executor.  If this is the first enqueue since the
    /// queue was drained, sets this executor's bit in the priority bitmap
    /// (via [`BitmapOps::set`]).
    /// Returns `true` if the task was actually enqueued.
    pub(crate) unsafe fn enqueue(&self, task: TaskRef) -> bool {
        let ops = unsafe { &*self.bitmap_ops.get() };
        task.info().state.run_enqueue(|cs| {
            log::trace!(
                "executor[{}]: enqueue task {:p}",
                self.priority,
                task.info()
            );
            task.info()
                .executor_ptr
                .store(core::ptr::from_ref(self).cast_mut(), Ordering::Release);
            self.run_queue.enqueue(task, cs);
            unsafe { (ops.set)(ops.ptr, self.priority, cs) };
        })
    }
}

/// Waker callback: re-enqueue a task on its owning executor.
///
/// Called from the waker vtable.  Loads the `executor_ptr` stored during
/// [`Executor::enqueue`] and delegates to [`Executor::enqueue`].
pub(crate) fn wake_task(task: TaskRef) {
    log::trace!("wake: task {:p}", task.info());
    let executor_ptr = task.info().executor_ptr.load(Ordering::Acquire);
    if !executor_ptr.is_null() {
        unsafe {
            if (*executor_ptr).enqueue(task) {
                platform::pend();
            }
        };
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::task::{Context, Poll, Waker};

    use crate::executor::{BitmapOps, Executor};
    use crate::spawner::SpawnToken;
    use crate::task::storage::TaskStorage;

    unsafe fn noop_bmp(_: *mut (), _: usize, _: critical_section::CriticalSection) {}

    fn new_exec(priority: usize) -> Executor {
        let exec = Executor::new(priority);
        exec.set_bitmap_ops(BitmapOps {
            ptr: core::ptr::null_mut(),
            set: noop_bmp,
            clear: noop_bmp,
        });
        exec
    }

    fn enqueue<F>(exec: &Executor, token: SpawnToken<F>) {
        let task = token.task_ref;
        core::mem::forget(token);
        unsafe { exec.enqueue(task) };
    }

    #[test]
    fn ready_future() {
        static TASK: TaskStorage<std::future::Ready<()>> = TaskStorage::new();
        let exec = new_exec(0);
        enqueue(&exec, TASK.spawn(|| std::future::ready(())).unwrap());
        exec.run();
    }

    #[test]
    fn pending_then_ready() {
        static POLL_COUNT: AtomicUsize = AtomicUsize::new(0);
        static TASK: TaskStorage<PendingOnce> = TaskStorage::new();

        struct PendingOnce;
        impl std::future::Future for PendingOnce {
            type Output = ();
            fn poll(self: std::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
                if POLL_COUNT.fetch_add(1, Ordering::Relaxed) == 0 {
                    cx.waker().wake_by_ref();
                    Poll::Pending
                } else {
                    Poll::Ready(())
                }
            }
        }

        let exec = new_exec(0);
        enqueue(&exec, TASK.spawn(|| PendingOnce).unwrap());
        exec.run();
        assert_eq!(POLL_COUNT.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn multiple_tasks() {
        static TASK_A: TaskStorage<CountN> = TaskStorage::new();
        static TASK_B: TaskStorage<CountN> = TaskStorage::new();
        static DONE: AtomicUsize = AtomicUsize::new(0);

        struct CountN;
        impl std::future::Future for CountN {
            type Output = ();
            fn poll(self: std::pin::Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
                DONE.fetch_add(1, Ordering::Relaxed);
                Poll::Ready(())
            }
        }

        let exec = new_exec(0);
        enqueue(&exec, TASK_A.spawn(|| CountN).unwrap());
        enqueue(&exec, TASK_B.spawn(|| CountN).unwrap());
        exec.run();
        assert_eq!(DONE.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn empty_run() {
        let exec = new_exec(0);
        exec.run();
    }

    #[test]
    fn multi_poll() {
        static POLLS: AtomicUsize = AtomicUsize::new(0);
        static TASK: TaskStorage<MultiPoll> = TaskStorage::new();

        struct MultiPoll;
        impl std::future::Future for MultiPoll {
            type Output = ();
            fn poll(self: std::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
                let n = POLLS.fetch_add(1, Ordering::Relaxed);
                if n < 4 {
                    cx.waker().wake_by_ref();
                    Poll::Pending
                } else {
                    Poll::Ready(())
                }
            }
        }

        let exec = new_exec(0);
        enqueue(&exec, TASK.spawn(|| MultiPoll).unwrap());
        exec.run();
        assert_eq!(POLLS.load(Ordering::Relaxed), 5);
    }

    #[test]
    fn inter_task_waking() {
        static FLAG: AtomicUsize = AtomicUsize::new(0);
        static WAKER: Mutex<Option<Waker>> = Mutex::new(None);
        static TASK_WAIT: TaskStorage<WaitFut> = TaskStorage::new();
        static TASK_SIGNAL: TaskStorage<SignalFut> = TaskStorage::new();

        struct WaitFut;
        struct SignalFut;

        impl std::future::Future for WaitFut {
            type Output = ();
            fn poll(self: std::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
                if FLAG.load(Ordering::Acquire) == 1 {
                    Poll::Ready(())
                } else {
                    *WAKER.lock().unwrap() = Some(cx.waker().clone());
                    Poll::Pending
                }
            }
        }

        impl std::future::Future for SignalFut {
            type Output = ();
            fn poll(self: std::pin::Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
                FLAG.store(1, Ordering::Release);
                if let Some(w) = WAKER.lock().unwrap().take() {
                    w.wake();
                }
                Poll::Ready(())
            }
        }

        let exec = new_exec(0);
        enqueue(&exec, TASK_WAIT.spawn(|| WaitFut).unwrap());
        enqueue(&exec, TASK_SIGNAL.spawn(|| SignalFut).unwrap());
        exec.run();
        assert_eq!(FLAG.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn pending_no_wake() {
        static POLLED: AtomicBool = AtomicBool::new(false);
        static TASK: TaskStorage<NeverReady> = TaskStorage::new();

        struct NeverReady;
        impl std::future::Future for NeverReady {
            type Output = ();
            fn poll(self: std::pin::Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
                POLLED.store(true, Ordering::Relaxed);
                Poll::Pending
            }
        }

        let exec = new_exec(0);
        enqueue(&exec, TASK.spawn(|| NeverReady).unwrap());
        exec.run();
        assert!(POLLED.load(Ordering::Relaxed));
    }

    #[test]
    fn respawn_after_completion() {
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

        let exec = new_exec(0);

        enqueue(&exec, TASK.spawn(|| CountOnce).unwrap());
        exec.run();
        assert_eq!(COUNT.load(Ordering::Relaxed), 1);

        enqueue(&exec, TASK.spawn(|| CountOnce).unwrap());
        exec.run();
        assert_eq!(COUNT.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn run_multiple_times() {
        static COUNT: AtomicUsize = AtomicUsize::new(0);
        static TASK_A: TaskStorage<CountRun> = TaskStorage::new();
        static TASK_B: TaskStorage<CountRun> = TaskStorage::new();

        struct CountRun;
        impl std::future::Future for CountRun {
            type Output = ();
            fn poll(self: std::pin::Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
                COUNT.fetch_add(1, Ordering::Relaxed);
                Poll::Ready(())
            }
        }

        let exec = new_exec(0);

        enqueue(&exec, TASK_A.spawn(|| CountRun).unwrap());
        exec.run();
        assert_eq!(COUNT.load(Ordering::Relaxed), 1);

        enqueue(&exec, TASK_B.spawn(|| CountRun).unwrap());
        exec.run();
        assert_eq!(COUNT.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn many_tasks() {
        static DONE: AtomicUsize = AtomicUsize::new(0);
        static T0: TaskStorage<Ct> = TaskStorage::new();
        static T1: TaskStorage<Ct> = TaskStorage::new();
        static T2: TaskStorage<Ct> = TaskStorage::new();
        static T3: TaskStorage<Ct> = TaskStorage::new();
        static T4: TaskStorage<Ct> = TaskStorage::new();
        static T5: TaskStorage<Ct> = TaskStorage::new();
        static T6: TaskStorage<Ct> = TaskStorage::new();
        static T7: TaskStorage<Ct> = TaskStorage::new();

        struct Ct;
        impl std::future::Future for Ct {
            type Output = ();
            fn poll(self: std::pin::Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
                DONE.fetch_add(1, Ordering::Relaxed);
                Poll::Ready(())
            }
        }

        let exec = new_exec(0);
        for t in [&T0, &T1, &T2, &T3, &T4, &T5, &T6, &T7] {
            enqueue(&exec, t.spawn(|| Ct).unwrap());
        }
        exec.run();
        assert_eq!(DONE.load(Ordering::Relaxed), 8);
    }
}
