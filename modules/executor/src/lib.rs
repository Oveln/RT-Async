//! # RT-Async Executor
//!
//! A `#![no_std]` async RTOS kernel with priority-preemptive scheduling.
//!
//! Each priority level has its own [`Executor`]; tasks of the same priority
//! cooperatively yield via `.await`, while higher-priority executors preempt
//! lower ones.  An O(1) two-level bitmap ([`PriorityBitmap`]) locates the
//! highest-priority ready executor in a single `trailing_zeros()` instruction.
//!
//! ## Architecture
//!
//! - [`Spawner`] owns `N` [`Executor`] instances and a shared
//!   [`PriorityBitmap`] wrapped in a [`critical_section::Mutex`].
//! - Each executor stores type-erased [`BitmapOps`] function pointers
//!   (injected during [`Spawner::init`]) so it can set/clear its bit in
//!   the scheduler bitmap without knowing the const-generic group count.
//! - [`TaskInfo`] holds per-task state: an atomic [`State`] machine, an
//!   intrusive [`RunQueueItem`], a back-pointer to the owning executor,
//!   and a poll function pointer.
//! - [`TaskStorage`] provides statically allocated future storage with
//!   spawn/despawn lifecycle management.
//!
//! ## Usage
//!
//! 1. Create a [`Spawner`] with the desired number of priorities.
//! 2. Pin it and call [`Spawner::init`] to wire up bitmap callbacks.
//! 3. Define tasks as [`TaskStorage`] statics, call [`TaskStorage::spawn`].
//! 4. Consume the returned [`SpawnToken`] via [`Spawner::spawn`].
//! 5. Call [`Spawner::try_preempt`] from interrupt handlers to check for
//!    higher-priority ready tasks.
//! 6. Execute the returned [`RunToken`] via [`Spawner::run`].
//!
//! [`Executor`]: crate::executor::Executor
//! [`BitmapOps`]: crate::executor::BitmapOps
//! [`TaskStorage`]: crate::task::storage::TaskStorage
//! [`TaskStorage::spawn`]: crate::task::storage::TaskStorage::spawn
//! [`TaskInfo`]: crate::task::header::TaskInfo
//! [`State`]: crate::task::state::State
//! [`RunQueueItem`]: crate::task::run_queue::RunQueueItem
//! [`SpawnToken`]: crate::spawner::SpawnToken
//! [`RunToken`]: crate::spawner::RunToken

#![no_std]
#![feature(sync_unsafe_cell)]
#![feature(unsafe_cell_access)]
#![allow(incomplete_features)]
#![feature(generic_const_exprs)]

pub use executor_macro::task;
pub use executor_macro::main;
pub use executor_macro::interrupt;

mod executor;
pub mod priority;
pub mod priority_bitmap;
pub mod spawner;
pub mod task;
mod util;
