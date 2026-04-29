use core::task::{RawWaker, RawWakerVTable, Waker};

use crate::{executor::wake_task, task::header::TaskRef};

/// Shared vtable for all task wakers.
///
/// The data pointer in each [`RawWaker`] is a `*const TaskInfo` (the task
/// header).  `clone` simply creates a new `RawWaker` with the same pointer —
/// tasks are statically allocated and never freed, so there is no reference
/// counting.
static VTABLE: RawWakerVTable = RawWakerVTable::new(clone, wake, wake, drop);

unsafe fn clone(p: *const ()) -> RawWaker {
    RawWaker::new(p, &VTABLE)
}

unsafe fn wake(p: *const ()) {
    let task = unsafe { TaskRef::from_ptr(p.cast()) };
    wake_task(task);
}

unsafe fn drop(_: *const ()) {}

/// Create a [`Waker`] for the given task.
///
/// # Safety
///
/// `p` must be a valid [`TaskRef`] obtained from a live [`TaskStorage`].
///
/// [`TaskStorage`]: crate::task::storage::TaskStorage
pub(crate) unsafe fn from_task(p: TaskRef) -> Waker {
    unsafe { Waker::from_raw(RawWaker::new(p.as_ptr() as _, &VTABLE)) }
}
