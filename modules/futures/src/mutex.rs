//! Async mutex for cooperative tasks.
//!
//! ```ignore
//! static DATA: Mutex<u32> = Mutex::new(0);
//!
//! async fn example() {
//!     let mut guard = DATA.lock().await;
//!     *guard += 1;
//! } // guard dropped → lock released
//! ```

use core::cell::UnsafeCell;
use core::future::Future;
use core::ops::{Deref, DerefMut};
use core::pin::Pin;
use core::task::{Context, Poll, Waker};

use critical_section::Mutex as CsMutex;
use heapless::Vec;

struct MutexInner<T, const N: usize> {
    locked: bool,
    value: T,
    /// Pointers to `Option<Waker>` stored inside each `MutexLockFuture`.
    waiters: Vec<*const Option<Waker>, N>,
}

/// Async mutex with a bounded FIFO waiter queue.
///
/// `N` is the maximum number of concurrent waiters (default 4). Exceeding this
/// will panic on `lock()`.
pub struct Mutex<T, const N: usize = 4> {
    inner: CsMutex<UnsafeCell<MutexInner<T, N>>>,
}

unsafe impl<T: Send, const N: usize> Sync for Mutex<T, N> {}
unsafe impl<T: Send, const N: usize> Send for Mutex<T, N> {}

impl<T, const N: usize> Mutex<T, N> {
    /// Create a new mutex in the unlocked state holding `value`.
    pub const fn new(value: T) -> Self {
        Self {
            inner: CsMutex::new(UnsafeCell::new(MutexInner {
                locked: false,
                value,
                waiters: Vec::new(),
            })),
        }
    }

    /// Acquire the lock asynchronously.
    pub fn lock(&self) -> MutexLockFuture<'_, T, N> {
        MutexLockFuture {
            mutex: self,
            queued: false,
            waker: None,
        }
    }
}

/// Future returned by [`Mutex::lock`].
pub struct MutexLockFuture<'a, T, const N: usize> {
    mutex: &'a Mutex<T, N>,
    queued: bool,
    waker: Option<Waker>,
}

impl<'a, T, const N: usize> Future for MutexLockFuture<'a, T, N> {
    type Output = MutexGuard<'a, T, N>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // SAFETY: we never move data out of self; only update `queued`/`waker`.
        let this = unsafe { self.get_unchecked_mut() };

        critical_section::with(|cs| {
            let inner = unsafe { &mut *this.mutex.inner.borrow(cs).get() };
            if !inner.locked {
                inner.locked = true;
                // Remove our entry from the waiter queue if we were queued.
                if this.queued {
                    let needle = &this.waker as *const _;
                    inner.waiters.retain(|&p| p != needle);
                    this.waker = None;
                }
                Poll::Ready(MutexGuard { mutex: this.mutex })
            } else {
                // Update the stored waker (re-poll with a new waker).
                this.waker = Some(cx.waker().clone());

                if !this.queued {
                    let ptr = &this.waker as *const _;
                    inner.waiters.push(ptr).expect("mutex: waiter queue full");
                    this.queued = true;
                }
                Poll::Pending
            }
        })
    }
}

impl<T, const N: usize> Drop for MutexLockFuture<'_, T, N> {
    fn drop(&mut self) {
        if self.queued {
            critical_section::with(|cs| {
                let inner = unsafe { &mut *self.mutex.inner.borrow(cs).get() };
                let needle = &self.waker as *const _;
                inner.waiters.retain(|&p| p != needle);
            });
        }
    }
}

/// RAII guard returned by `lock().await`. Releases the lock and wakes the next
/// waiter on drop.
pub struct MutexGuard<'a, T, const N: usize> {
    mutex: &'a Mutex<T, N>,
}

impl<T, const N: usize> Deref for MutexGuard<'_, T, N> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        critical_section::with(|cs| {
            let inner = unsafe { &*self.mutex.inner.borrow(cs).get() };
            &inner.value
        })
    }
}

impl<T, const N: usize> DerefMut for MutexGuard<'_, T, N> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        critical_section::with(|cs| {
            let inner = unsafe { &mut *self.mutex.inner.borrow(cs).get() };
            &mut inner.value
        })
    }
}

impl<T, const N: usize> Drop for MutexGuard<'_, T, N> {
    fn drop(&mut self) {
        critical_section::with(|cs| {
            let inner = unsafe { &mut *self.mutex.inner.borrow(cs).get() };
            inner.locked = false;
            if !inner.waiters.is_empty() {
                let ptr = inner.waiters.remove(0);
                // SAFETY: the pointer points into a pinned MutexLockFuture
                // that is still alive (it hasn't completed yet).
                let waker = unsafe { &*ptr };
                if let Some(w) = waker.as_ref() {
                    w.wake_by_ref();
                }
            }
        });
    }
}
