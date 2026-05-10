//! Async primitives for rt-async.
//!
//! The [`timer`] module provides an async sleep API backed by the platform's
//! [`TimerChip`] implementation.

#![no_std]

pub mod timer;
