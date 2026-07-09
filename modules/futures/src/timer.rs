//! Async timer API backed by the platform's [`platform::Timer`] driver.
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
//!     futures::timer::handle_timer_isr();
//! }
//! ```

use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll, Waker};

use fugit::Duration;

/// Global timer queue (capacity 16).
///
/// Slots are shared between async `after()` futures and the timer-sched
/// ISR-driven action scheduler.  16 entries × 24 bytes = 384 bytes total.
pub static TIMER_QUEUE: ::timer::TimerQueue<16> = ::timer::TimerQueue::new();

/// Nanosecond-precision duration type.
pub type TimerDuration = Duration<u64, 1, 1_000_000_000>;

/// Wait for `duration` to elapse.
///
/// Accepts a nanosecond-precision [`TimerDuration`]; fugit's `.millis()` /
/// `.micros()` / `.secs()` helpers infer the correct type automatically.
///
/// ```ignore
/// use fugit::ExtU64;
/// timer::after(1.millis()).await;
/// timer::after(500.micros()).await;
/// ```
pub fn after(duration: TimerDuration) -> TimerDelay {
    let hw_freq = platform::timer().freq_hz() as u128;
    let ticks = (duration.as_ticks() as u128 * hw_freq / 1_000_000_000) as u64;
    let deadline = platform::timer().now().saturating_add(ticks);
    TimerDelay {
        deadline,
        registered: false,
        waker: None,
    }
}

/// Future returned by [`after`]. Completes when `now >= deadline`.
pub struct TimerDelay {
    deadline: u64,
    registered: bool,
    waker: Option<Waker>,
}

impl Future for TimerDelay {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if platform::timer().now() >= self.deadline {
            self.registered = false;
            Poll::Ready(())
        } else {
            if !self.registered {
                self.waker = Some(cx.waker().clone());
                let data = core::ptr::addr_of_mut!(self.waker) as *mut ();
                let is_earliest =
                    unsafe { TIMER_QUEUE.schedule(self.deadline, wake_trampoline, data) };
                if is_earliest {
                    platform::timer().set_deadline(self.deadline);
                }
                self.registered = true;
            }
            Poll::Pending
        }
    }
}

impl Drop for TimerDelay {
    fn drop(&mut self) {
        if self.registered {
            let data = core::ptr::addr_of_mut!(self.waker) as *mut ();
            TIMER_QUEUE.cancel(data);
            self.registered = false;
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
    platform::timer().set_deadline(u64::MAX);
    let next = TIMER_QUEUE.dequeue_expired(platform::timer().now());
    if let Some(d) = next {
        platform::timer().set_deadline(d);
    }
}
