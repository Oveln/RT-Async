//! Async serial RX backed by the NS16550A driver's ring buffer.
//!
//! # Usage
//!
//! Requires that the board has registered the UART RX IRQ handler (done in
//! `Board::init`/`Board::late_init`):
//! ```ignore
//! let byte = futures::serial::SerialRx::new().await;
//! ```
//!
//! ```ignore
//! let byte = futures::serial::SerialRx::new().await;
//! ```

use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

/// Future that completes when a byte is received from the serial port.
///
/// Uses the NS16550A driver's built-in ring buffer and waker slot.
/// If no RX IRQ is registered, this will never complete — use
/// `platform::console().read()` for polling instead.
pub struct SerialRx;

impl SerialRx {
    pub fn new() -> Self {
        Self
    }
}

impl Future for SerialRx {
    type Output = u8;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<u8> {
        platform::drivers::serial_ns16550a::rx_poll(cx)
    }
}
