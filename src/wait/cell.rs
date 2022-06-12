use super::Status;
use crate::util::{CachePadded, UnsafeDeref};

use std::cell::UnsafeCell;
use std::sync::atomic::fence;
use std::sync::atomic::AtomicU8;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::task::Waker;

pub struct Cell {
    state: AtomicU8,
    waker: UnsafeCell<Option<Waker>>,
}

unsafe impl Send for Cell {}
unsafe impl Sync for Cell {}

const WAITING: u8 = 0b000;
const WOKE: u8 = 0b001;
const REGISTERING: u8 = 0b010;
const WAKING: u8 = 0b100;

impl Cell {
    pub fn new() -> Cell {
        Cell {
            state: AtomicU8::new(WAITING),
            waker: UnsafeCell::new(None),
        }
    }

    pub unsafe fn register(&self, waker: &Waker) -> Status {
        let mut state = self.state.load(Ordering::Relaxed);

        if state == WAITING {
            let will_wake = self
                .waker
                .get()
                .deref()
                .as_ref()
                .map(|w| w.will_wake(waker))
                .unwrap_or(false);

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
                            return Status::Registered;
                        }

                        fence(Ordering::Acquire);
                        return Status::Awoke;
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
                Ok(_) => return Status::Awoke,
                Err(found) => state = found,
            }
        }

        if state == WOKE {
            fence(Ordering::Acquire);
            self.state.store(WAITING, Ordering::Relaxed);
            return Status::Awoke;
        }

        match state {
            WAKING => Status::Awoke,
            _ => Status::Registered,
        }
    }

    pub fn wake(&self) {
        let mut state = self.state.load(Ordering::Relaxed);

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
