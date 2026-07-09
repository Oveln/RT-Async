//! Async serial RX backed by the registry's console device.
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

use platform::SerialRxStatus;

/// Future that completes when a byte is received from the serial port.
///
/// Polls RX through the registry's console [`platform::Serial`] device via
/// [`platform::Serial::rx_register_waker`]. Drivers that support interrupt
/// driven RX (e.g. NS16550A) implement that method with the
/// disable→register→recheck→enable critical-section pattern; drivers that do
/// not (including host stubs) return [`SerialRxStatus::Unsupported`], in which
/// case this Future never completes — use `platform::console().read()` for
/// polling instead.
pub struct SerialRx;

impl SerialRx {
    pub fn new() -> Self {
        Self
    }
}

impl Future for SerialRx {
    type Output = u8;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<u8> {
        match platform::driver::console().rx_register_waker(cx) {
            SerialRxStatus::Ready(b) => Poll::Ready(b),
            SerialRxStatus::Pending | SerialRxStatus::Unsupported => Poll::Pending,
        }
    }
}
