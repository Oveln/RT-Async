use core::{cell::UnsafeCell, ptr::addr_of_mut};

use cordyceps::{Linked, List, list::Links};

use crate::task::header::{TaskInfo, TaskRef};

pub(crate) type RunQueueItem = Links<TaskInfo>;

unsafe impl Linked<Links<TaskInfo>> for TaskInfo {
    type Handle = TaskRef;

    fn into_ptr(r: Self::Handle) -> core::ptr::NonNull<Self> {
        r.ptr
    }

    unsafe fn from_ptr(ptr: core::ptr::NonNull<Self>) -> Self::Handle {
        TaskRef { ptr }
    }

    unsafe fn links(ptr: core::ptr::NonNull<Self>) -> core::ptr::NonNull<Links<Self>> {
        let ptr: *mut TaskInfo = ptr.as_ptr();
        unsafe { core::ptr::NonNull::new_unchecked(addr_of_mut!((*ptr).run_queue_item)) }
    }
}

/// Intrusive FIFO run queue for tasks sharing the same priority.
///
/// Internally wraps a [`cordyceps::List`] behind a
/// `critical_section::Mutex<UnsafeCell<...>>` so that enqueue/dequeue can be
/// called from interrupt handlers as well as the executor run loop.
pub(crate) struct RunQueue {
    inner: critical_section::Mutex<UnsafeCell<List<TaskInfo>>>,
}

impl RunQueue {
    pub(crate) const fn new() -> Self {
        Self {
            inner: critical_section::Mutex::new(UnsafeCell::new(List::new())),
        }
    }
    pub(crate) fn dequeue(&self, cs: critical_section::CriticalSection) -> Option<TaskRef> {
        let queue = unsafe { &mut *self.inner.borrow(cs).get() };
        let task = queue.pop_front();
        match task {
            Some(task) => {
                task.info().state.run_dequeue();
                Some(task)
            }
            None => None,
        }
    }
    pub(crate) fn enqueue(&self, task: TaskRef, cs: critical_section::CriticalSection) {
        let queue = unsafe { &mut *self.inner.borrow(cs).get() };
        queue.push_back(task);
    }
}
