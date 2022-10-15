use super::signal::Signal;
use crate::raw::util::UnsafeDeref;

use std::cell::UnsafeCell;
use std::sync::atomic::{fence, AtomicU8, Ordering};
use std::task::Poll;
use std::task::Waker;

/// An asynchronous task that can be parked/unparked.
#[derive(Default)]
pub struct Task {
    state: Signal<AtomicU8>,
    waker: UnsafeCell<Option<Waker>>,
}

unsafe impl Send for Task {}
unsafe impl Sync for Task {}

/// Asynchronously 'block' until a resource is ready, parking the
/// task if it is not.
///
// This is a macro for better inlining behavior (which seems
// hit and miss with async fns)
macro_rules! block_on {
    ($task:expr => || $poll:expr) => {{
        $crate::raw::util::poll_fn(|cx| loop {
            if let Poll::Ready(value) = { $poll } {
                return Poll::Ready(value);
            }

            match unsafe { $task.register(cx.waker()) } {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(_) => ::std::hint::spin_loop(),
            }
        })
        .await
    }};
}

pub(crate) use block_on;

// the task is parked
const PARKED: u8 = 0b000;
// the task was unparked
const UNPARKED: u8 = 0b001;
// the task is registering a waker
const REGISTERING: u8 = 0b010;
// someone is currently waking the task
const WAKING: u8 = 0b100;

impl Task {
    /// Create a new `Task`.
    pub fn new() -> Task {
        Task {
            state: Signal::new(PARKED),
            waker: UnsafeCell::new(None),
        }
    }

    /// Register a waker to be unparked.
    ///
    /// `Poll::Pending` indicates the waker was successfully registered
    /// and the task should park.
    ///
    /// `Poll::Ready` indicates the unpark token was set and the resource
    /// should be checked again.
    ///
    /// # Safety
    ///
    /// This method must only be called from a single thread.
    pub unsafe fn register(&self, waker: &Waker) -> Poll<()> {
        // acquire: synchronize with the unpark token
        let mut state = self.state.load_acquire();

        // if the unpark token is not set, we have to register the waker
        if state == PARKED {
            let will_wake = {
                // safety: the waker is always safe to dereference
                // because we are the sole producer, and consumers only
                // have read-only access
                let current = self.waker.get().deref();
                current.as_ref().filter(|c| c.will_wake(waker)).is_some()
            };

            // if the current waker will wake this task, we can skip re-registering it
            if will_wake {
                return Poll::Pending;
            }

            // try to acquire the registration lock
            match self.state.inner().compare_exchange(
                state,
                state | REGISTERING,
                Ordering::Acquire,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    // safety: we hold the lock
                    *self.waker.get() = Some(waker.clone());

                    // release the lock
                    let current = self.state.inner().swap(PARKED, Ordering::Release);

                    match current {
                        // if nothing happened in the meantime, we succesfully
                        // registered
                        REGISTERING => return Poll::Pending,
                        // otherwise someone tried to wake us
                        _ => {
                            assert_eq!(current, (REGISTERING | UNPARKED));
                            // synchronize with the unpark token
                            fence(Ordering::Acquire);
                            return Poll::Ready(());
                        }
                    }
                }
                // failed to acquire the lock, someone is trying to wake the old task
                Err(found) => {
                    assert!(
                        found == (UNPARKED | WAKING) || found == UNPARKED,
                        "{found:08b}"
                    );
                    state = found
                }
            }
        }

        // if the unpark token is set and someone is currently waking
        // the old task, try to eagerly consume the token and return
        // ready
        if state == (WAKING | UNPARKED) {
            match self.state.inner().compare_exchange(
                state,
                // we must retaing the WAKING flag for safety
                // of future registrations
                state - UNPARKED,
                Ordering::Acquire,
                Ordering::Acquire,
            ) {
                Ok(_) => return Poll::Ready(()),
                Err(found) => {
                    // the task finished waking
                    assert_eq!(found, UNPARKED);
                    state = found
                }
            }
        }

        // if the unpark token is set but the waker is *not* being
        // woken, we can just consume the token and return ready
        if state == UNPARKED {
            // we can safetly reset the state here because:
            // - we are the only consumer
            // - any producers will see UNPARKED and leave the state as is
            self.state.inner().store(PARKED, Ordering::Relaxed);
            return Poll::Ready(());
        }

        // if we eagerly consumed the UNPARKED flag in a previous call
        // to register, but the producer is still waking the waker,
        // we have to retry again
        if state == WAKING {
            return Poll::Ready(());
        }

        // there are no other possible states, this should be unreachable
        unreachable!()
    }

    /// Unpark the task.
    ///
    /// This method will set a token that can be consumed by calls to [`register`],
    /// and wake the current registered waker.
    pub fn unpark(&self) {
        let mut state = self.state.load_release();

        loop {
            // the unpark token is already set, we're done. the future consumer
            // will also see the token and retry. the release ordering also
            // ensures that our push is not lost.
            if state & UNPARKED != 0 {
                return;
            }

            // set the UNPARKED token
            let mut new = state | UNPARKED;

            // if the task is currently registering a new waker
            // they will see our token, so we don't have to wake
            if state & REGISTERING == 0 {
                // otherwise we have to acquire the waking lock
                new |= WAKING
            }

            match self.state.inner().compare_exchange_weak(
                state,
                new,
                Ordering::Release,
                Ordering::Relaxed,
            ) {
                // set the token and acquired the lock, now we have to wake
                Ok(PARKED) => {
                    // acquire REGISTERING
                    fence(Ordering::Acquire);

                    // safety: we hold the waking lock
                    let waker = unsafe { self.waker.get().deref().clone() };

                    // release the lock
                    self.state.inner().fetch_sub(WAKING, Ordering::Release);

                    if let Some(waker) = waker {
                        waker.wake();
                    }
                    return;
                }
                // the task is still registering, but they will see our token
                Ok(_) => return,
                // something changed, retry
                Err(found) => state = found,
            }
        }
    }
}
