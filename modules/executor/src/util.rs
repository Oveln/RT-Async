use core::cell::UnsafeCell;
use core::mem::MaybeUninit;
use core::ptr;

/// Storage cell that starts uninitialized and is written exactly once.
///
/// Combines [`MaybeUninit`] (deferred initialization) with [`UnsafeCell`]
/// (interior mutability) so that a `&UninitCell<T>` can be used to write
/// and later drop `T` in place — required for futures stored in `static`
/// [`TaskStorage`].
///
/// # Safety
///
/// This type is `Sync` so that it can live in `static`s shared across
/// threads/cores.  The caller is responsible for ensuring that writes and
/// reads are properly synchronized (e.g. via [`critical_section`]).
///
/// [`TaskStorage`]: crate::task::storage::TaskStorage
pub(crate) struct UninitCell<T>(MaybeUninit<UnsafeCell<T>>);
impl<T> UninitCell<T> {
    pub const fn uninit() -> Self {
        Self(MaybeUninit::uninit())
    }

    /// Returns a raw mutable pointer to the inner `T`.
    ///
    /// # Safety
    ///
    /// The caller must ensure the cell has been initialized before reading
    /// through this pointer, and that no aliasing `&mut T` exists.
    #[allow(clippy::mut_from_ref)]
    pub unsafe fn as_mut_ptr(&self) -> *mut T {
        unsafe { (*self.0.as_ptr()).get() }
    }

    /// Returns a mutable reference to the inner `T`.
    ///
    /// # Safety
    ///
    /// Same constraints as [`as_mut_ptr`](Self::as_mut_ptr).  Additionally,
    /// the caller must not call this concurrently with any other mutable
    /// access.
    #[allow(clippy::mut_from_ref)]
    pub unsafe fn as_mut(&self) -> &mut T {
        unsafe { &mut *self.as_mut_ptr() }
    }

    /// Initializes the cell by writing the result of `func` into it.
    ///
    /// # Safety
    ///
    /// Must only be called once.  Calling `write_in_place` on an already-
    /// initialized cell leaks the previous value and may cause a double-drop.
    pub unsafe fn write_in_place(&self, func: impl FnOnce() -> T) {
        unsafe { ptr::write(self.as_mut_ptr(), func()) }
    }

    /// Drops the inner value in place.
    ///
    /// # Safety
    ///
    /// Caller must ensure the value has been initialized and will not be
    /// accessed again.
    pub unsafe fn drop_in_place(&self) {
        unsafe { ptr::drop_in_place(self.as_mut_ptr()) }
    }
}

unsafe impl<T> Sync for UninitCell<T> {}
