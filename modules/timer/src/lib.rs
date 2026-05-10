//! Timer callback queue for ISR-driven delayed execution.
//!
//! [`TimerQueue`] stores `(deadline, callback, data)` triples. The timer ISR
//! calls [`TimerQueue::dequeue_expired`] to fire all callbacks whose deadline
//! has passed.
//!
//! # ISR integration
//!
//! ```ignore
//! #[executor::interrupt]
//! fn MachineTimer(_tf: &mut TrapFrame) {
//!     Chip::set_deadline(u64::MAX);
//!     let next = TIMER_QUEUE.dequeue_expired(Chip::now_ticks());
//!     if let Some(d) = next {
//!         Chip::set_deadline(d);
//!     }
//! }
//! ```

#![no_std]

use core::cell::UnsafeCell;

use critical_section::Mutex as CsMutex;
use heapless::Vec;

/// Callback type: `unsafe fn(data)` invoked from ISR context.
pub type Callback = unsafe fn(*mut ());

struct Entry {
    deadline: u64,
    callback: Callback,
    data: *mut (),
}

/// Fixed-capacity deadline queue shared between tasks and the timer ISR.
///
/// `N` is the maximum number of concurrently pending timers.
pub struct TimerQueue<const N: usize> {
    inner: CsMutex<UnsafeCell<Vec<Entry, N>>>,
}

unsafe impl<const N: usize> Send for TimerQueue<N> {}
unsafe impl<const N: usize> Sync for TimerQueue<N> {}

impl<const N: usize> TimerQueue<N> {
    /// Create an empty queue (const-compatible).
    pub const fn new() -> Self {
        Self {
            inner: CsMutex::new(UnsafeCell::new(Vec::new())),
        }
    }

    /// Schedule `callback(data)` to be invoked no earlier than `deadline` ticks.
    ///
    /// Returns `true` if `deadline` is the new earliest in the queue — the
    /// caller should reprogram the hardware compare register accordingly.
    ///
    /// # Safety
    ///
    /// `data` must remain valid until the callback fires or the entry is
    /// cancelled. The callback will be called in ISR context.
    pub unsafe fn schedule(&self, deadline: u64, callback: Callback, data: *mut ()) -> bool {
        critical_section::with(|cs| {
            let entries = unsafe { &mut *self.inner.borrow(cs).get() };
            let prev_min = entries.iter().map(|e| e.deadline).min();
            if entries
                .push(Entry {
                    deadline,
                    callback,
                    data,
                })
                .is_err()
            {
                panic!("timer: TimerQueue full");
            }
            let new_min = entries.iter().map(|e| e.deadline).min().unwrap();
            prev_min.map_or(true, |p| new_min < p)
        })
    }

    /// Fire all callbacks whose deadline ≤ `now`, then return the next
    /// earliest remaining deadline (or `None` if the queue is empty).
    pub fn dequeue_expired(&self, now: u64) -> Option<u64> {
        critical_section::with(|cs| {
            let entries = unsafe { &mut *self.inner.borrow(cs).get() };
            let mut i = 0;
            while i < entries.len() {
                if entries[i].deadline <= now {
                    let entry = entries.swap_remove(i);
                    // SAFETY: callback and data were registered via `schedule`.
                    unsafe { (entry.callback)(entry.data) };
                } else {
                    i += 1;
                }
            }
            entries.iter().map(|e| e.deadline).min()
        })
    }
}
