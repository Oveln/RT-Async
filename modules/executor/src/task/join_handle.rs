use core::{
    future::Future,
    marker::PhantomData,
    pin::Pin,
    task::{Context, Poll},
};

use crate::task::header::TaskRef;

/// A future that resolves to the output of a spawned task.
///
/// Returned by [`Spawner::spawn`] when a task is dispatched to an executor.
/// Awaiting this handle yields the future's `T` once the task completes.
///
/// [`Spawner::spawn`]: crate::spawner::Spawner::spawn
pub struct JoinHandle<T> {
    task: TaskRef,
    _marker: PhantomData<T>,
}

impl<T> JoinHandle<T> {
    pub(crate) fn new(task: TaskRef) -> Self {
        Self {
            task,
            _marker: PhantomData,
        }
    }
}

impl<T> Future for JoinHandle<T> {
    type Output = T;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<T> {
        // SAFETY: T == F::Output (guaranteed by Spawner::spawn).
        // No concurrent access to result/waker because the spawned task
        // and JoinHandle::poll cannot execute simultaneously:
        // - Higher priority: task already completed during spawn
        //   (preempted inside Spawner::spawn), so result is Some.
        // - Lower or equal priority: cannot preempt this poll.
        unsafe {
            let join = self.task.join_state::<T>();
            match (*join.result.get()).take() {
                Some(value) => Poll::Ready(value),
                None => {
                    *join.waker.get() = Some(cx.waker().clone());
                    Poll::Pending
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

    use super::*;
    use crate::priority::Priority;
    use crate::spawner::Spawner;
    use crate::task::storage::TaskStorage;

    fn noop_waker() -> Waker {
        static VTABLE: RawWakerVTable = RawWakerVTable::new(
            |_: *const ()| RawWaker::new(core::ptr::null(), &VTABLE),
            |_: *const ()| {},
            |_: *const ()| {},
            |_: *const ()| {},
        );
        unsafe { Waker::from_raw(RawWaker::new(core::ptr::null(), &VTABLE)) }
    }

    macro_rules! pinned_spawner {
        ($name:ident, $n:expr) => {
            let $name = Spawner::<$n>::new();
            let mut $name = core::pin::pin!($name);
            $name.as_mut().init();
            let $name = $name;
        };
    }

    #[test]
    fn join_handle_ready_immediately() {
        static TASK: TaskStorage<std::future::Ready<u32>> = TaskStorage::new();

        pinned_spawner!(spawner, 4);
        let handle = spawner.as_ref().spawn(
            Priority::new(0),
            TASK.spawn(|| std::future::ready(42u32)).unwrap(),
        );

        // Run the executor to completion
        while let Some(rt) = spawner.as_ref().try_preempt() {
            spawner.as_ref().run(rt);
            spawner.as_ref().complete_executor();
        }

        // JoinHandle should resolve immediately
        let waker = noop_waker();
        let mut cx = Context::from_waker(&waker);
        let mut handle = handle;
        let result = Pin::new(&mut handle).poll(&mut cx);
        assert_eq!(result, Poll::Ready(42));
    }

    #[test]
    fn join_handle_pending_then_ready() {
        static POLLS: AtomicUsize = AtomicUsize::new(0);
        static TASK: TaskStorage<PendingOnce> = TaskStorage::new();

        struct PendingOnce;
        impl std::future::Future for PendingOnce {
            type Output = u32;
            fn poll(self: std::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<u32> {
                let n = POLLS.fetch_add(1, Ordering::Relaxed);
                if n == 0 {
                    cx.waker().wake_by_ref();
                    Poll::Pending
                } else {
                    Poll::Ready(99)
                }
            }
        }

        pinned_spawner!(spawner, 4);
        let handle = spawner
            .as_ref()
            .spawn(Priority::new(0), TASK.spawn(|| PendingOnce).unwrap());

        // Poll JoinHandle before task completes
        let waker = noop_waker();
        let mut cx = Context::from_waker(&waker);
        let mut handle = handle;
        assert_eq!(Pin::new(&mut handle).poll(&mut cx), Poll::Pending);

        // Run the executor
        while let Some(rt) = spawner.as_ref().try_preempt() {
            spawner.as_ref().run(rt);
            spawner.as_ref().complete_executor();
        }

        // Now JoinHandle should be ready
        let result = Pin::new(&mut handle).poll(&mut cx);
        assert_eq!(result, Poll::Ready(99));
    }

    #[test]
    fn join_handle_value_correctness() {
        static TASK: TaskStorage<std::future::Ready<&'static str>> = TaskStorage::new();

        pinned_spawner!(spawner, 4);
        let handle = spawner.as_ref().spawn(
            Priority::new(0),
            TASK.spawn(|| std::future::ready("hello")).unwrap(),
        );

        while let Some(rt) = spawner.as_ref().try_preempt() {
            spawner.as_ref().run(rt);
            spawner.as_ref().complete_executor();
        }

        let waker = noop_waker();
        let mut cx = Context::from_waker(&waker);
        let mut handle = handle;
        let result = Pin::new(&mut handle).poll(&mut cx);
        assert_eq!(result, Poll::Ready("hello"));
    }
}
