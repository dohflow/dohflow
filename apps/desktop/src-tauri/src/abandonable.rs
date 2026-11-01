//! A call that cannot be interrupted, run so a cancel can stop waiting for it
//! (personal-cfo-pxi.25; ADR 0060 addendum 2026-10-31).
//!
//! A provider fetch is a blocking network call with no cancellation hook, so a cancel cannot stop
//! it, only stop waiting for it. [`run_abandonable`] runs the call on its own thread and polls a
//! cancellation check at the job runtime's poll interval. On a request it returns at once and the
//! call is abandoned: when it ends, its result is dropped (never delivered), everything it holds
//! is dropped with it, and `on_abandoned_end` runs on that thread, which is where a caller
//! releases whatever the abandoned call still owned.

use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex, MutexGuard};

use finance_kernel::CANCELLATION_POLL_INTERVAL;

/// How a call ended, from the caller's side.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum CallEnd<T> {
    /// The call returned; this is its result.
    Done(T),
    /// A cancel was seen first. The call is abandoned and its result will be dropped.
    Cancelled,
    /// The call's thread could not start or panicked; there is no result.
    Lost,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Running,
    /// The caller stopped waiting. The thread owns the cleanup.
    Abandoned,
    /// The result was handed over.
    Finished,
}

fn locked(phase: &Mutex<Phase>) -> MutexGuard<'_, Phase> {
    phase
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Runs `on_abandoned_end` when the call's thread ends while abandoned, whether it returned or
/// panicked.
struct EndGuard<F: FnOnce()> {
    phase: Arc<Mutex<Phase>>,
    on_abandoned_end: Option<F>,
}

impl<F: FnOnce()> Drop for EndGuard<F> {
    fn drop(&mut self) {
        if *locked(&self.phase) == Phase::Abandoned {
            if let Some(release) = self.on_abandoned_end.take() {
                release();
            }
        }
    }
}

/// Run `work` on its own thread and wait for it, polling `cancelled` every poll interval.
///
/// The caller owns the decision to stop waiting; the call is never interrupted. A result that is
/// produced after the caller stopped waiting is dropped by the thread and can never reach the
/// caller, so it cannot reach whatever commits next.
pub(crate) fn run_abandonable<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
    mut cancelled: impl FnMut() -> bool,
    on_abandoned_end: impl FnOnce() + Send + 'static,
) -> CallEnd<T> {
    let phase = Arc::new(Mutex::new(Phase::Running));
    let (sender, receiver) = mpsc::channel();
    let thread_phase = Arc::clone(&phase);
    let spawned = std::thread::Builder::new()
        .name("provider-call".to_owned())
        .spawn(move || {
            let _end = EndGuard {
                phase: Arc::clone(&thread_phase),
                on_abandoned_end: Some(on_abandoned_end),
            };
            let result = work();
            let mut phase = locked(&thread_phase);
            if *phase == Phase::Abandoned {
                // Nobody is waiting: drop the result here, with the thread's other buffers.
                drop(phase);
                drop(result);
            } else {
                *phase = Phase::Finished;
                let _ = sender.send(result);
            }
        });
    if spawned.is_err() {
        return CallEnd::Lost;
    }
    loop {
        match receiver.recv_timeout(CANCELLATION_POLL_INTERVAL) {
            Ok(result) => return CallEnd::Done(result),
            Err(RecvTimeoutError::Disconnected) => return CallEnd::Lost,
            Err(RecvTimeoutError::Timeout) => {}
        }
        if cancelled() {
            let mut phase = locked(&phase);
            if *phase == Phase::Finished {
                // The result arrived while the check ran: take it, the caller's own check before
                // the commit decides what a request means now.
                drop(phase);
                continue;
            }
            *phase = Phase::Abandoned;
            return CallEnd::Cancelled;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::sync::Condvar;
    use std::time::{Duration, Instant};

    use super::*;

    /// A call that blocks until released, and says when it started.
    struct Gate {
        entered: AtomicBool,
        open: (Mutex<bool>, Condvar),
    }

    impl Gate {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                entered: AtomicBool::new(false),
                open: (Mutex::new(false), Condvar::new()),
            })
        }
        fn wait(&self) {
            self.entered.store(true, Ordering::SeqCst);
            let (lock, opened) = &self.open;
            let mut open = lock.lock().unwrap();
            while !*open {
                open = opened.wait(open).unwrap();
            }
        }
        fn release(&self) {
            let (lock, opened) = &self.open;
            *lock.lock().unwrap() = true;
            opened.notify_all();
        }
    }

    fn eventually(mut condition: impl FnMut() -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if condition() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        false
    }

    #[test]
    fn a_call_that_returns_is_delivered_and_nothing_is_released() {
        let released = Arc::new(AtomicU32::new(0));
        let counter = Arc::clone(&released);
        let end = run_abandonable(
            || 7,
            || false,
            move || {
                counter.fetch_add(1, Ordering::SeqCst);
            },
        );
        assert_eq!(end, CallEnd::Done(7));
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(
            released.load(Ordering::SeqCst),
            0,
            "not abandoned, not released"
        );
    }

    #[test]
    fn a_cancel_returns_without_waiting_for_a_blocked_call() {
        let gate = Gate::new();
        let worker_gate = Arc::clone(&gate);
        let started = Instant::now();
        let end = run_abandonable(
            move || {
                worker_gate.wait();
                1
            },
            {
                let gate = Arc::clone(&gate);
                move || gate.entered.load(Ordering::SeqCst)
            },
            || {},
        );
        let took = started.elapsed();
        assert_eq!(end, CallEnd::Cancelled);
        assert!(took < Duration::from_millis(500), "returned after {took:?}");
        gate.release();
    }

    #[test]
    fn an_abandoned_calls_result_is_dropped_and_its_cleanup_runs_once_when_it_ends() {
        struct Dropped(Arc<AtomicU32>);
        impl Drop for Dropped {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
        let gate = Gate::new();
        let worker_gate = Arc::clone(&gate);
        let dropped = Arc::new(AtomicU32::new(0));
        let released = Arc::new(AtomicU32::new(0));
        let worker_dropped = Arc::clone(&dropped);
        let counter = Arc::clone(&released);
        let end = run_abandonable(
            move || {
                worker_gate.wait();
                Dropped(worker_dropped)
            },
            {
                let gate = Arc::clone(&gate);
                move || gate.entered.load(Ordering::SeqCst)
            },
            move || {
                counter.fetch_add(1, Ordering::SeqCst);
            },
        );
        assert!(matches!(end, CallEnd::Cancelled));
        assert_eq!(
            released.load(Ordering::SeqCst),
            0,
            "the slot is held while the call still runs"
        );
        gate.release();
        assert!(eventually(|| released.load(Ordering::SeqCst) == 1));
        assert_eq!(
            dropped.load(Ordering::SeqCst),
            1,
            "the late result was dropped"
        );
    }

    #[test]
    fn a_panicking_call_is_lost_and_an_abandoned_panic_still_releases() {
        let end: CallEnd<u8> = run_abandonable(|| panic!("provider"), || false, || {});
        assert_eq!(end, CallEnd::Lost);

        let gate = Gate::new();
        let worker_gate = Arc::clone(&gate);
        let released = Arc::new(AtomicU32::new(0));
        let counter = Arc::clone(&released);
        let end: CallEnd<u8> = run_abandonable(
            move || {
                worker_gate.wait();
                panic!("provider");
            },
            {
                let gate = Arc::clone(&gate);
                move || gate.entered.load(Ordering::SeqCst)
            },
            move || {
                counter.fetch_add(1, Ordering::SeqCst);
            },
        );
        assert_eq!(end, CallEnd::Cancelled);
        gate.release();
        assert!(eventually(|| released.load(Ordering::SeqCst) == 1));
    }
}
