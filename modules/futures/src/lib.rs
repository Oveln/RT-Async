//! Async primitives for rt-async.
//!
//! The [`timer`] module provides an async sleep API backed by the platform's
//! [`Timer`] driver. [`serial`] provides async UART byte receive.

#![no_std]

pub mod mutex;
pub mod serial;
pub mod timer;
