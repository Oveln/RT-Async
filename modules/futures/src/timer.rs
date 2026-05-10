//! Async timer API for the active platform.
//!
//! ```ignore
//! use fugit::ExtU64;
//! futures::timer::after(1.millis()).await;
//! ```
//!
//! # ISR integration
//!
//! ```ignore
//! #[executor::interrupt]
//! fn MachineTimer(_tf: &mut TrapFrame) {
//!     ChipImpl::set_deadline(u64::MAX);
//!     let next = futures::timer::TIMER_QUEUE.dequeue_expired(ChipImpl::now_ticks());
//!     if let Some(d) = next {
//!         ChipImpl::set_deadline(d);
//!     }
//! }
//! ```

use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll, Waker};

use fugit::Duration;
use platform::platform_traits::timer::TimerChip;
use platform::ChipImpl;

/// Global timer queue (capacity 8).
pub static TIMER_QUEUE: ::timer::TimerQueue<8> = ::timer::TimerQueue::new();

/// Duration type matching the active chip's tick frequency.
pub type TimerDuration = Duration<u64, 1, { ChipImpl::FREQ_HZ as u64 }>;

/// Wait for `duration` to elapse.
///
/// Accepts any fugit `Duration<u64, ...>` and converts to ticks at
/// `ChipImpl::FREQ_HZ`. Use with [`fugit::ExtU64`]:
///
/// ```ignore
/// use fugit::ExtU64;
/// timer::after(1.millis()).await;
/// timer::after(500.micros()).await;
/// ```
pub fn after(duration: Duration<u64, 1, { ChipImpl::FREQ_HZ as u64 }>) -> TimerDelay {
    let ticks: u64 = duration
        .as_ticks();
    let deadline = ChipImpl::now_ticks().saturating_add(ticks);
    TimerDelay {
        deadline,
        registered: false,
        waker: None,
    }
}

/// Future returned by [`after`]. Completes when `ChipImpl::now_ticks() >= deadline`.
pub struct TimerDelay {
    deadline: u64,
    registered: bool,
    waker: Option<Waker>,
}

impl Future for TimerDelay {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if ChipImpl::now_ticks() >= self.deadline {
            Poll::Ready(())
        } else {
            if !self.registered {
                self.waker = Some(cx.waker().clone());
                let data = core::ptr::addr_of_mut!(self.waker) as *mut ();
                let is_earliest = unsafe { TIMER_QUEUE.schedule(self.deadline, wake_trampoline, data) };
                if is_earliest {
                    ChipImpl::set_deadline(self.deadline);
                }
                self.registered = true;
            }
            Poll::Pending
        }
    }
}

/// Trampoline: reads the `Option<Waker>` stored in a [`TimerDelay`] and wakes it.
unsafe fn wake_trampoline(data: *mut ()) {
    let waker = unsafe { &*(data as *const Option<Waker>) };
    if let Some(w) = waker.as_ref() {
        w.wake_by_ref();
    }
}

/// Timer ISR handler.
///
/// Call this from the platform's `MachineTimer` interrupt:
///
/// ```ignore
/// #[executor::interrupt]
/// fn MachineTimer(_tf: &mut TrapFrame) {
///     futures::timer::handle_timer_isr();
/// }
/// ```
pub fn handle_timer_isr() {
    ChipImpl::set_deadline(u64::MAX);
    let next = TIMER_QUEUE.dequeue_expired(ChipImpl::now_ticks());
    if let Some(d) = next {
        ChipImpl::set_deadline(d);
    }
}
