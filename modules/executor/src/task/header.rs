use core::{cell::SyncUnsafeCell, ptr::NonNull};

use portable_atomic::AtomicPtr;

use crate::{
    executor::Executor,
    task::{run_queue::RunQueueItem, state::State},
};

pub(crate) struct TaskInfo {
    pub(crate) state: State,
    pub(crate) run_queue_item: RunQueueItem,
    /// Back-pointer to the [`Executor`] that owns this task.
    /// `null` if the task has not yet been enqueued or has completed.
    /// Stored with [`Release`](Ordering::Release) ordering during
    /// [`enqueue`](Executor::enqueue) and loaded with [`Acquire`](Ordering::Acquire)
    /// in [`wake_task`].
    pub(crate) executor_ptr: AtomicPtr<Executor>,
    /// Function pointer for polling this task's future.
    /// Set to [`TaskStorage::poll`] on spawn; replaced with `poll_exited`
    /// (a no-op) once the future returns `Ready`.
    pub(crate) poll_fn: SyncUnsafeCell<Option<unsafe fn(TaskRef)>>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TaskRef {
    pub(crate) ptr: NonNull<TaskInfo>,
}

unsafe impl Send for TaskRef where &'static TaskInfo: Send {}
unsafe impl Sync for TaskRef where &'static TaskInfo: Sync {}

impl TaskRef {
    pub(crate) fn info(&self) -> &'static TaskInfo {
        unsafe { self.ptr.as_ref() }
    }

    pub(crate) fn as_ptr(self) -> *const TaskInfo {
        self.ptr.as_ptr()
    }

    pub(crate) unsafe fn from_ptr(ptr: *const TaskInfo) -> Self {
        Self {
            ptr: unsafe { NonNull::new_unchecked(ptr.cast_mut()) },
        }
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::format;

    fn make_task_info() -> &'static TaskInfo {
        static INFO: TaskInfo = TaskInfo {
            state: State::new(),
            run_queue_item: RunQueueItem::new(),
            executor_ptr: AtomicPtr::new(core::ptr::null_mut()),
            poll_fn: SyncUnsafeCell::new(None),
        };
        &INFO
    }

    #[test]
    fn task_ref_from_ptr_roundtrip() {
        let info = make_task_info();
        let ptr = info as *const TaskInfo;
        let task_ref = unsafe { TaskRef::from_ptr(ptr) };
        assert_eq!(task_ref.as_ptr(), ptr);
    }

    #[test]
    fn task_ref_info_returns_same() {
        let info = make_task_info();
        let ptr = info as *const TaskInfo;
        let task_ref = unsafe { TaskRef::from_ptr(ptr) };
        let info2 = task_ref.info();
        assert!(core::ptr::eq(info, info2));
    }

    #[test]
    fn task_ref_equality() {
        let info = make_task_info();
        let ptr = info as *const TaskInfo;
        let r1 = unsafe { TaskRef::from_ptr(ptr) };
        let r2 = unsafe { TaskRef::from_ptr(ptr) };
        assert_eq!(r1, r2);
    }

    #[test]
    fn task_ref_clone() {
        let info = make_task_info();
        let ptr = info as *const TaskInfo;
        let r1 = unsafe { TaskRef::from_ptr(ptr) };
        let r2 = r1.clone();
        assert_eq!(r1, r2);
    }

    #[test]
    fn task_ref_copy() {
        let info = make_task_info();
        let ptr = info as *const TaskInfo;
        let r1 = unsafe { TaskRef::from_ptr(ptr) };
        let r2 = r1;
        assert_eq!(r1, r2);
    }

    #[test]
    fn task_ref_debug_format() {
        let info = make_task_info();
        let ptr = info as *const TaskInfo;
        let r = unsafe { TaskRef::from_ptr(ptr) };
        let _s = format!("{:?}", r);
    }
}
