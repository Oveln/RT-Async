use core::{
    cell::UnsafeCell,
    marker::{PhantomData, PhantomPinned},
    pin::Pin,
};

use heapless::Vec;

use crate::{
    executor::{BitmapOps, Executor},
    priority::Priority,
    priority_bitmap::PriorityBitmap,
    task::TaskRef,
};

pub const fn ceil_div(a: usize, b: usize) -> usize {
    (a + b - 1) / b
}

type BitmapMutex<const G: usize> = critical_section::Mutex<UnsafeCell<PriorityBitmap<G>>>;

/// Opaque handle representing a spawned but not-yet-scheduled task.
///
/// **Must-consume semantics**: dropping a `SpawnToken` without passing it to
/// [`Spawner::spawn`] will panic.  This prevents accidentally leaking tasks
/// that are never scheduled.
pub struct SpawnToken<F> {
    pub(crate) task_ref: TaskRef,
    phantom: PhantomData<*mut F>,
}

impl<F> SpawnToken<F> {
    pub(crate) fn new(task_ref: TaskRef) -> Self {
        Self {
            task_ref,
            phantom: PhantomData,
        }
    }
}

impl<F> Drop for SpawnToken<F> {
    fn drop(&mut self) {
        panic!("SpawnToken must be consumed by Spawner::spawn, not dropped");
    }
}

/// Token returned by [`Spawner::try_preempt`] indicating which priority
/// executor should run next.
pub struct RunToken(Priority);

impl RunToken {
    pub(crate) const fn new(prio: Priority) -> Self {
        Self(prio)
    }
}

/// Priority-preemptive task scheduler.
///
/// `N` is the number of priority levels (1..=4096).
///
/// All executors share a single system stack.  When a higher-priority
/// executor preempts a lower one, the new executor's stack usage simply
/// grows on top — like a nested function call.  When it finishes, execution
/// unwinds back to the preempted executor, naturally reclaiming the stack.
///
/// The priority stack (`prio_stack`) tracks which executors currently own
/// stack space, from oldest (bottom) to newest (top).  The executor at the
/// top of the stack is the one currently running.
///
/// Internally backed by a [`PriorityBitmap`] with `ceil(N / 64)` groups,
/// wrapped in a [`critical_section::Mutex`] so that both the executors
/// (from ISR/context) and [`try_preempt`](Self::try_preempt) access it
/// safely without races.
///
/// # Initialization
///
/// `Spawner` is `!Unpin` — after construction, it must be pinned, then
/// [`init`](Self::init) must be called once before any other method.
/// [`init`](Self::init) injects type-erased bitmap operation callbacks
/// ([`BitmapOps`]) into each executor so they can signal the bitmap
/// without knowing its concrete const-generic group count.
///
/// ```ignore
/// let mut spawner = Spawner::<4>::new();
/// let mut spawner = pin!(spawner);
/// spawner.as_mut().init();
/// ```
///
/// [`BitmapOps`]: crate::executor::BitmapOps
pub struct Spawner<const N: usize>
where
    [(); ceil_div(N, 64)]:,
{
    executors: [Executor; N],
    bitmap: BitmapMutex<{ ceil_div(N, 64) }>,
    prio_stack: critical_section::Mutex<UnsafeCell<Vec<Priority, N>>>,
    _pinned: PhantomPinned,
}

impl<const N: usize> Spawner<N>
where
    [(); ceil_div(N, 64)]:,
{
    const GROUPS: usize = ceil_div(N, 64);
    const _ASSERT_N_IN_RANGE: () = assert!(
        N > 0 && Self::GROUPS <= 64,
        "Spawner<N>: N must be in 1..=4096"
    );

    unsafe fn bm_set(ctx: *mut (), prio: usize, cs: critical_section::CriticalSection) {
        let mutex = unsafe { &*(ctx as *const BitmapMutex<{ ceil_div(N, 64) }>) };
        unsafe { (*mutex.borrow(cs).get()).set(prio) };
    }

    unsafe fn bm_clear(ctx: *mut (), prio: usize, cs: critical_section::CriticalSection) {
        let mutex = unsafe { &*(ctx as *const BitmapMutex<{ ceil_div(N, 64) }>) };
        unsafe { (*mutex.borrow(cs).get()).clear(prio) };
    }

    pub fn new() -> Self {
        let () = Self::_ASSERT_N_IN_RANGE;
        Self {
            executors: core::array::from_fn(Executor::new),
            bitmap: critical_section::Mutex::new(UnsafeCell::new(PriorityBitmap::new())),
            prio_stack: critical_section::Mutex::new(UnsafeCell::new(Vec::new())),
            _pinned: PhantomPinned,
        }
    }

    /// Wire each executor to the scheduler's priority bitmap.
    ///
    /// Creates [`BitmapOps`] containing a raw pointer to the `Mutex`-wrapped
    /// bitmap and monomorphised `set`/`clear` function pointers, then injects
    /// them into every executor.  Must be called exactly once after pinning.
    ///
    /// # Safety context
    ///
    /// The `Pin` guarantee ensures the bitmap's address remains stable for
    /// the `Spawner`'s entire lifetime, so the raw pointer stored in each
    /// [`BitmapOps`] is always valid.
    ///
    /// [`BitmapOps`]: crate::executor::BitmapOps
    pub fn init(self: Pin<&mut Self>) {
        let this = unsafe { self.get_unchecked_mut() };
        let ctx = core::ptr::from_ref(&this.bitmap) as *mut ();
        for exec in &this.executors {
            exec.set_bitmap_ops(BitmapOps {
                ptr: ctx,
                set: Self::bm_set,
                clear: Self::bm_clear,
            });
        }
    }

    /// Dispatch a spawned task to the executor at the given priority.
    ///
    /// Consumes the [`SpawnToken`] (via `mem::forget`) to satisfy the
    /// must-consume contract.
    pub fn spawn<S: Send>(self: Pin<&Self>, prio: Priority, token: SpawnToken<S>) {
        let executor = self
            .executors
            .get(prio.to_usize())
            .expect("priority out of range");
        let task = token.task_ref;
        core::mem::forget(token);
        unsafe {
            executor.enqueue(task);
        }
    }

    /// Check the priority bitmap (inside the `Mutex`) for a higher-priority
    /// ready executor.
    ///
    /// Reads the currently-running priority from the top of `prio_stack`.
    /// If the highest ready priority is higher than the current one (or the
    /// stack is empty — initial execution), pushes the new priority onto the
    /// stack and returns a [`RunToken`] for it.
    ///
    /// Returns `None` if the current executor is already the highest-priority
    /// ready executor.
    pub fn try_preempt(self: Pin<&Self>) -> Option<RunToken> {
        critical_section::with(|cs| {
            let stack = unsafe { self.prio_stack.borrow(cs).as_mut_unchecked() };
            let bitmap = unsafe { &*self.bitmap.borrow(cs).get() };
            let highest_prio = bitmap.highest()?;
            let highest = Priority::new(highest_prio);
            match stack.last().copied() {
                None => {
                    stack.push(highest).ok()?;
                    Some(RunToken::new(highest))
                }
                Some(p) if p.is_lower_than(&highest) => {
                    stack.push(highest).ok()?;
                    Some(RunToken::new(highest))
                }
                Some(_) => None,
            }
        })
    }

    /// Run the executor associated with `run_token`.
    ///
    /// Executes all ready tasks on that executor's run queue in a loop,
    /// polling each task's future until the queue is empty.
    pub fn run(self: Pin<&Self>, run_token: RunToken) {
        self.executors[run_token.0.to_usize()].run();
    }

    /// Pop the just-completed executor's priority from the priority stack.
    ///
    /// Must be called after [`run`](Self::run) returns.  This signals that
    /// the executor has reclaimed its stack space and the next call to
    /// [`try_preempt`](Self::try_preempt) should use the previous (preempted)
    /// executor's priority as the current one.
    pub fn complete_executor(self: Pin<&Self>) {
        critical_section::with(|cs| {
            let stack = unsafe { self.prio_stack.borrow(cs).as_mut_unchecked() };
            stack.pop();
        });
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::task::{Context, Poll};

    use super::*;
    use crate::priority::Priority;
    use crate::task::storage::TaskStorage;

    macro_rules! pinned_spawner {
        ($name:ident, $n:expr) => {
            let $name = Spawner::<$n>::new();
            let mut $name = core::pin::pin!($name);
            $name.as_mut().init();
            let $name = $name;
        };
    }

    #[test]
    #[should_panic(expected = "SpawnToken must be consumed")]
    fn spawn_token_dropped() {
        static TASK: TaskStorage<std::future::Ready<()>> = TaskStorage::new();
        let _token = TASK.spawn(|| std::future::ready(())).unwrap();
    }

    #[test]
    fn spawner_new_creates_empty() {
        pinned_spawner!(spawner, 4);
        assert!(spawner.as_ref().try_preempt().is_none());
    }

    #[test]
    fn spawn_and_run_single_task() {
        static DONE: AtomicUsize = AtomicUsize::new(0);
        static TASK: TaskStorage<DoneOnce> = TaskStorage::new();

        struct DoneOnce;
        impl std::future::Future for DoneOnce {
            type Output = ();
            fn poll(self: std::pin::Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
                DONE.fetch_add(1, Ordering::Relaxed);
                Poll::Ready(())
            }
        }

        pinned_spawner!(spawner, 4);
        spawner
            .as_ref()
            .spawn(Priority::new(0), TASK.spawn(|| DoneOnce).unwrap());

        let rt = spawner.as_ref().try_preempt().unwrap();
        spawner.as_ref().run(rt);
        spawner.as_ref().complete_executor();
        assert_eq!(DONE.load(Ordering::Relaxed), 1);
        assert!(spawner.as_ref().try_preempt().is_none());
    }

    #[test]
    fn spawn_multiple_priorities() {
        static DONE: AtomicUsize = AtomicUsize::new(0);

        struct Done;
        impl std::future::Future for Done {
            type Output = ();
            fn poll(self: std::pin::Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
                DONE.fetch_add(1, Ordering::Relaxed);
                Poll::Ready(())
            }
        }

        static T0: TaskStorage<Done> = TaskStorage::new();
        static T1: TaskStorage<Done> = TaskStorage::new();
        static T2: TaskStorage<Done> = TaskStorage::new();

        pinned_spawner!(spawner, 4);
        spawner
            .as_ref()
            .spawn(Priority::new(0), T0.spawn(|| Done).unwrap());
        spawner
            .as_ref()
            .spawn(Priority::new(1), T1.spawn(|| Done).unwrap());
        spawner
            .as_ref()
            .spawn(Priority::new(2), T2.spawn(|| Done).unwrap());

        let s = spawner.as_ref();
        while let Some(rt) = s.try_preempt() {
            s.run(rt);
            s.complete_executor();
        }
        assert_eq!(DONE.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn preempt_higher_priority_first() {
        static ORDER: std::sync::Mutex<std::vec::Vec<usize>> = std::sync::Mutex::new(std::vec![]);

        struct RecordOrder(usize);
        impl std::future::Future for RecordOrder {
            type Output = ();
            fn poll(self: std::pin::Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
                ORDER.lock().unwrap().push(self.0);
                Poll::Ready(())
            }
        }

        static T_LOW: TaskStorage<RecordOrder> = TaskStorage::new();
        static T_HIGH: TaskStorage<RecordOrder> = TaskStorage::new();

        pinned_spawner!(spawner, 4);
        spawner
            .as_ref()
            .spawn(Priority::new(2), T_LOW.spawn(|| RecordOrder(2)).unwrap());
        spawner
            .as_ref()
            .spawn(Priority::new(0), T_HIGH.spawn(|| RecordOrder(0)).unwrap());

        let s = spawner.as_ref();
        while let Some(rt) = s.try_preempt() {
            s.run(rt);
            s.complete_executor();
        }
        let order = ORDER.lock().unwrap();
        assert_eq!(*order, std::vec![0, 2]);
    }

    #[test]
    fn complete_executor_pops_stack() {
        struct Done;
        impl std::future::Future for Done {
            type Output = ();
            fn poll(self: std::pin::Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
                Poll::Ready(())
            }
        }

        static T0: TaskStorage<Done> = TaskStorage::new();
        static T1: TaskStorage<Done> = TaskStorage::new();

        pinned_spawner!(spawner, 4);
        spawner
            .as_ref()
            .spawn(Priority::new(0), T0.spawn(|| Done).unwrap());
        spawner
            .as_ref()
            .spawn(Priority::new(1), T1.spawn(|| Done).unwrap());

        let rt = spawner.as_ref().try_preempt().unwrap();
        assert_eq!(rt.0.to_usize(), 0);
        spawner.as_ref().run(rt);
        spawner.as_ref().complete_executor();

        let rt = spawner.as_ref().try_preempt().unwrap();
        assert_eq!(rt.0.to_usize(), 1);
        spawner.as_ref().run(rt);
        spawner.as_ref().complete_executor();

        assert!(spawner.as_ref().try_preempt().is_none());
    }

    #[test]
    fn no_preempt_when_current_is_highest() {
        struct PendingForever;
        impl std::future::Future for PendingForever {
            type Output = ();
            fn poll(self: std::pin::Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
                Poll::Pending
            }
        }
        struct Done;
        impl std::future::Future for Done {
            type Output = ();
            fn poll(self: std::pin::Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
                Poll::Ready(())
            }
        }

        static T_HIGH: TaskStorage<PendingForever> = TaskStorage::new();
        static T_LOW: TaskStorage<Done> = TaskStorage::new();

        pinned_spawner!(spawner, 4);
        spawner
            .as_ref()
            .spawn(Priority::new(0), T_HIGH.spawn(|| PendingForever).unwrap());
        spawner
            .as_ref()
            .spawn(Priority::new(2), T_LOW.spawn(|| Done).unwrap());

        let rt = spawner.as_ref().try_preempt().unwrap();
        assert_eq!(rt.0.to_usize(), 0);
        spawner.as_ref().run(rt);
        spawner.as_ref().complete_executor();

        let rt = spawner.as_ref().try_preempt().unwrap();
        assert_eq!(rt.0.to_usize(), 2);
        spawner.as_ref().run(rt);
        spawner.as_ref().complete_executor();

        assert!(spawner.as_ref().try_preempt().is_none());
    }

    #[test]
    fn wake_from_task() {
        static FLAG: AtomicUsize = AtomicUsize::new(0);
        static WAKER_STORE: std::sync::Mutex<Option<std::task::Waker>> =
            std::sync::Mutex::new(None);

        struct WaitFlag;
        impl std::future::Future for WaitFlag {
            type Output = ();
            fn poll(self: std::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
                if FLAG.load(Ordering::Acquire) == 1 {
                    Poll::Ready(())
                } else {
                    *WAKER_STORE.lock().unwrap() = Some(cx.waker().clone());
                    Poll::Pending
                }
            }
        }

        static TASK: TaskStorage<WaitFlag> = TaskStorage::new();

        FLAG.store(0, Ordering::Relaxed);
        pinned_spawner!(spawner, 4);
        spawner
            .as_ref()
            .spawn(Priority::new(0), TASK.spawn(|| WaitFlag).unwrap());

        let rt = spawner.as_ref().try_preempt().unwrap();
        spawner.as_ref().run(rt);
        spawner.as_ref().complete_executor();

        FLAG.store(1, Ordering::Relaxed);
        if let Some(w) = WAKER_STORE.lock().unwrap().take() {
            w.wake();
        }

        let rt = spawner.as_ref().try_preempt().unwrap();
        spawner.as_ref().run(rt);
        spawner.as_ref().complete_executor();

        assert!(spawner.as_ref().try_preempt().is_none());
    }

    #[test]
    fn single_priority_spawner() {
        static DONE: AtomicUsize = AtomicUsize::new(0);

        struct Done;
        impl std::future::Future for Done {
            type Output = ();
            fn poll(self: std::pin::Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
                DONE.fetch_add(1, Ordering::Relaxed);
                Poll::Ready(())
            }
        }

        static T: TaskStorage<Done> = TaskStorage::new();
        pinned_spawner!(spawner, 1);
        spawner
            .as_ref()
            .spawn(Priority::new(0), T.spawn(|| Done).unwrap());

        let rt = spawner.as_ref().try_preempt().unwrap();
        spawner.as_ref().run(rt);
        spawner.as_ref().complete_executor();
        assert_eq!(DONE.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn respawn_and_re_run() {
        static COUNT: AtomicUsize = AtomicUsize::new(0);

        struct Count;
        impl std::future::Future for Count {
            type Output = ();
            fn poll(self: std::pin::Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
                COUNT.fetch_add(1, Ordering::Relaxed);
                Poll::Ready(())
            }
        }

        static TASK: TaskStorage<Count> = TaskStorage::new();
        pinned_spawner!(spawner, 4);

        let s = spawner.as_ref();
        for _ in 0..3 {
            let token = TASK.spawn(|| Count).unwrap();
            s.spawn(Priority::new(0), token);
            let rt = s.try_preempt().unwrap();
            s.run(rt);
            s.complete_executor();
        }
        assert_eq!(COUNT.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn inter_task_wake_across_priorities() {
        static SIGNALED: AtomicUsize = AtomicUsize::new(0);
        static WAKER_STORE: std::sync::Mutex<Option<std::task::Waker>> =
            std::sync::Mutex::new(None);

        struct Waiter;
        impl std::future::Future for Waiter {
            type Output = ();
            fn poll(self: std::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
                if SIGNALED.load(Ordering::Acquire) == 1 {
                    Poll::Ready(())
                } else {
                    *WAKER_STORE.lock().unwrap() = Some(cx.waker().clone());
                    Poll::Pending
                }
            }
        }

        struct Signaler;
        impl std::future::Future for Signaler {
            type Output = ();
            fn poll(self: std::pin::Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
                SIGNALED.store(1, Ordering::Release);
                if let Some(w) = WAKER_STORE.lock().unwrap().take() {
                    w.wake();
                }
                Poll::Ready(())
            }
        }

        static T_WAIT: TaskStorage<Waiter> = TaskStorage::new();
        static T_SIGNAL: TaskStorage<Signaler> = TaskStorage::new();

        pinned_spawner!(spawner, 4);
        spawner
            .as_ref()
            .spawn(Priority::new(0), T_WAIT.spawn(|| Waiter).unwrap());
        spawner
            .as_ref()
            .spawn(Priority::new(1), T_SIGNAL.spawn(|| Signaler).unwrap());

        let rt = spawner.as_ref().try_preempt().unwrap();
        assert_eq!(rt.0.to_usize(), 0);
        spawner.as_ref().run(rt);
        spawner.as_ref().complete_executor();

        let rt = spawner.as_ref().try_preempt().unwrap();
        assert_eq!(rt.0.to_usize(), 1);
        spawner.as_ref().run(rt);
        spawner.as_ref().complete_executor();

        let rt = spawner.as_ref().try_preempt().unwrap();
        assert_eq!(rt.0.to_usize(), 0);
        spawner.as_ref().run(rt);
        spawner.as_ref().complete_executor();

        assert!(spawner.as_ref().try_preempt().is_none());
        assert_eq!(SIGNALED.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn try_preempt_returns_none_when_bitmap_clear() {
        pinned_spawner!(spawner, 4);
        assert!(spawner.as_ref().try_preempt().is_none());
    }

    #[test]
    fn max_priorities_spawner() {
        static DONE: AtomicUsize = AtomicUsize::new(0);

        struct Done;
        impl std::future::Future for Done {
            type Output = ();
            fn poll(self: std::pin::Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
                DONE.fetch_add(1, Ordering::Relaxed);
                Poll::Ready(())
            }
        }

        static T0: TaskStorage<Done> = TaskStorage::new();
        static T63: TaskStorage<Done> = TaskStorage::new();

        pinned_spawner!(spawner, 64);
        spawner
            .as_ref()
            .spawn(Priority::new(63), T63.spawn(|| Done).unwrap());
        spawner
            .as_ref()
            .spawn(Priority::new(0), T0.spawn(|| Done).unwrap());

        let rt = spawner.as_ref().try_preempt().unwrap();
        assert_eq!(rt.0.to_usize(), 0);
        spawner.as_ref().run(rt);
        spawner.as_ref().complete_executor();

        let rt = spawner.as_ref().try_preempt().unwrap();
        assert_eq!(rt.0.to_usize(), 63);
        spawner.as_ref().run(rt);
        spawner.as_ref().complete_executor();

        assert!(spawner.as_ref().try_preempt().is_none());
        assert_eq!(DONE.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn run_token_new_priority() {
        let rt = RunToken::new(Priority::new(42));
        assert_eq!(rt.0.to_usize(), 42);
    }
}
