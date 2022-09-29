use crate::raw::util::UnsafeDeref;

use std::cell::UnsafeCell;
use std::sync::atomic::fence;
use std::sync::atomic::AtomicU8;
use std::sync::atomic::Ordering;
use std::task::Poll;
use std::task::Waker;

/// An asynchronous task that can be parked/unparked.
#[derive(Default)]
pub struct Task {
    state: AtomicU8,
    waker: UnsafeCell<Option<Waker>>,
}

unsafe impl Send for Task {}
unsafe impl Sync for Task {}

const WAITING: u8 = 0b000;
const WOKE: u8 = 0b001;
const REGISTERING: u8 = 0b010;
const WAKING: u8 = 0b100;

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

            match unsafe { $task.poll(cx.waker()) } {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(_) => ::std::hint::spin_loop(),
            }
        })
        .await
    }};
}

pub(crate) use block_on;

impl Task {
    pub fn new() -> Task {
        Task {
            state: AtomicU8::new(WAITING),
            waker: UnsafeCell::new(None),
        }
    }

    pub unsafe fn poll(&self, waker: &Waker) -> Poll<()> {
        let mut state = self.state.load(Ordering::Relaxed);

        if state == WAITING {
            let will_wake = {
                let current = self.waker.get().deref();
                current.as_ref().filter(|c| c.will_wake(waker)).is_some()
            };

            if !will_wake {
                match self.state.compare_exchange(
                    state,
                    state | REGISTERING,
                    Ordering::Acquire,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => {
                        *self.waker.get() = Some(waker.clone());
                        if self.state.swap(WAITING, Ordering::Release) == REGISTERING {
                            return Poll::Ready(());
                        }

                        fence(Ordering::Acquire);
                        return Poll::Ready(());
                    }
                    Err(found) => state = found,
                }
            }
        }

        if state == (WAKING | WOKE) {
            match self.state.compare_exchange(
                state,
                state - WOKE,
                Ordering::Acquire,
                Ordering::Relaxed,
            ) {
                Ok(_) => return Poll::Ready(()),
                Err(found) => state = found,
            }
        }

        if state == WOKE {
            fence(Ordering::Acquire);
            self.state.store(WAITING, Ordering::Relaxed);
            return Poll::Ready(());
        }

        match state {
            WAKING => Poll::Ready(()),
            _ => Poll::Pending,
        }
    }

    /// Unpark the task.
    pub fn unpark(&self) {
        let mut state = self.state.load(Ordering::SeqCst);

        loop {
            if state & WOKE != 0 {
                return;
            }

            let mut new = state | WOKE;
            if state & REGISTERING == 0 {
                new |= WAKING
            }

            match self
                .state
                .compare_exchange_weak(state, new, Ordering::Release, Ordering::Relaxed)
            {
                Ok(WAITING) => {
                    fence(Ordering::Acquire);
                    let waker = unsafe { self.waker.get().deref().clone() };
                    self.state.fetch_sub(WAKING, Ordering::Release);

                    if let Some(waker) = waker {
                        waker.wake();
                    }

                    return;
                }
                Ok(_) => return,
                Err(found) => state = found,
            }
        }
    }
}
