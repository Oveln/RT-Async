use core::sync::atomic::Ordering;

type StateAtomic = portable_atomic::AtomicU8;
type StateBits = u8;

pub(crate) struct State {
    inner: StateAtomic,
}

pub(crate) const STATE_SPAWNED: StateBits = 1 << 0;

pub(crate) const STATE_RUN_QUEUED: StateBits = 1 << 1;

impl State {
    pub const fn new() -> Self {
        Self {
            inner: StateAtomic::new(0),
        }
    }

    /// Atomically mark the task as spawned.
    ///
    /// Returns `true` if the task was not previously spawned (first call),
    /// `false` if it was already spawned (re-spawn attempt).
    pub fn spawn(&self) -> bool {
        let prev = self.inner.fetch_or(STATE_SPAWNED, Ordering::AcqRel);
        prev & STATE_SPAWNED == 0
    }

    /// Atomically clear the spawned flag.
    pub fn despawn(&self) {
        self.inner.fetch_and(!STATE_SPAWNED, Ordering::AcqRel);
    }

    /// Atomically try to enqueue the task into its executor's run queue.
    ///
    /// Uses `fetch_or` + early return to avoid entering the critical section
    /// if the task is already enqueued (common case on double-wake).
    /// Only calls `f` inside `critical_section::with` when the `STATE_RUN_QUEUED`
    /// flag was previously clear.
    /// Returns `true` if the task was actually enqueued (first enqueue),
    /// `false` if it was already queued (skipped).
    pub fn run_enqueue(&self, f: impl FnOnce(critical_section::CriticalSection)) -> bool {
        let prev = self.inner.fetch_or(STATE_RUN_QUEUED, Ordering::AcqRel);
        if prev & STATE_RUN_QUEUED == 0 {
            critical_section::with(f);
            true
        } else {
            false
        }
    }

    /// Atomically clear the run-queued flag.
    pub fn run_dequeue(&self) {
        self.inner.fetch_and(!STATE_RUN_QUEUED, Ordering::AcqRel);
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;

    #[test]
    fn new_state_is_zero() {
        let state = State::new();
        let val = state.inner.load(Ordering::Relaxed);
        assert_eq!(val, 0);
    }

    #[test]
    fn spawn_first_time_returns_true() {
        let state = State::new();
        assert!(state.spawn());
    }

    #[test]
    fn spawn_second_time_returns_false() {
        let state = State::new();
        assert!(state.spawn());
        assert!(!state.spawn());
    }

    #[test]
    fn spawn_multiple_times() {
        let state = State::new();
        assert!(state.spawn());
        assert!(!state.spawn());
        assert!(!state.spawn());
    }

    #[test]
    fn despawn_after_spawn() {
        let state = State::new();
        state.spawn();
        state.despawn();
        assert!(state.spawn(), "Should be spawnable again after despawn");
    }

    #[test]
    fn despawn_when_not_spawned_is_harmless() {
        let state = State::new();
        state.despawn();
        assert!(state.spawn());
    }

    #[test]
    fn spawn_despawn_cycle() {
        let state = State::new();
        for _ in 0..3 {
            assert!(state.spawn());
            state.despawn();
        }
    }

    #[test]
    fn run_enqueue_calls_closure_on_first_call() {
        use std::sync::atomic::AtomicUsize;
        static CALLED: AtomicUsize = AtomicUsize::new(0);
        CALLED.store(0, Ordering::Relaxed);

        let state = State::new();
        state.run_enqueue(|_| {
            CALLED.fetch_add(1, Ordering::Relaxed);
        });
        assert_eq!(CALLED.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn run_enqueue_skips_closure_on_second_call() {
        use std::sync::atomic::AtomicUsize;
        static CALLED: AtomicUsize = AtomicUsize::new(0);
        CALLED.store(0, Ordering::Relaxed);

        let state = State::new();
        state.run_enqueue(|_| {
            CALLED.fetch_add(1, Ordering::Relaxed);
        });
        state.run_enqueue(|_| {
            CALLED.fetch_add(1, Ordering::Relaxed);
        });
        assert_eq!(
            CALLED.load(Ordering::Relaxed),
            1,
            "Closure should only be called once"
        );
    }

    #[test]
    fn run_dequeue_allows_re_enqueue() {
        use std::sync::atomic::AtomicUsize;
        static CALLED: AtomicUsize = AtomicUsize::new(0);
        CALLED.store(0, Ordering::Relaxed);

        let state = State::new();
        state.run_enqueue(|_| {
            CALLED.fetch_add(1, Ordering::Relaxed);
        });
        state.run_dequeue();
        state.run_enqueue(|_| {
            CALLED.fetch_add(1, Ordering::Relaxed);
        });
        assert_eq!(CALLED.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn run_enqueue_multiple_without_dequeue() {
        use std::sync::atomic::AtomicUsize;
        static CALLED: AtomicUsize = AtomicUsize::new(0);
        CALLED.store(0, Ordering::Relaxed);

        let state = State::new();
        for _ in 0..5 {
            state.run_enqueue(|_| {
                CALLED.fetch_add(1, Ordering::Relaxed);
            });
        }
        assert_eq!(CALLED.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn state_bits_spawned_flag() {
        let state = State::new();
        state.spawn();
        let val = state.inner.load(Ordering::Relaxed);
        assert_ne!(val & STATE_SPAWNED, 0);
    }

    #[test]
    fn state_bits_run_queued_flag() {
        let state = State::new();
        state.run_enqueue(|_| {});
        let val = state.inner.load(Ordering::Relaxed);
        assert_ne!(val & STATE_RUN_QUEUED, 0);
    }

    #[test]
    fn both_flags_set() {
        let state = State::new();
        state.spawn();
        state.run_enqueue(|_| {});
        let val = state.inner.load(Ordering::Relaxed);
        assert_ne!(val & STATE_SPAWNED, 0);
        assert_ne!(val & STATE_RUN_QUEUED, 0);
    }

    #[test]
    fn clear_spawn_preserves_run_queued() {
        let state = State::new();
        state.spawn();
        state.run_enqueue(|_| {});
        state.despawn();
        let val = state.inner.load(Ordering::Relaxed);
        assert_eq!(val & STATE_SPAWNED, 0, "SPAWNED should be cleared");
        assert_ne!(val & STATE_RUN_QUEUED, 0, "RUN_QUEUED should still be set");
    }

    #[test]
    fn clear_run_queued_preserves_spawned() {
        let state = State::new();
        state.spawn();
        state.run_enqueue(|_| {});
        state.run_dequeue();
        let val = state.inner.load(Ordering::Relaxed);
        assert_ne!(val & STATE_SPAWNED, 0, "SPAWNED should still be set");
        assert_eq!(val & STATE_RUN_QUEUED, 0, "RUN_QUEUED should be cleared");
    }
}
