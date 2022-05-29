use std::cell::UnsafeCell;
use std::sync::atomic::fence;
use std::sync::atomic::AtomicU8;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::task::Waker;

use crate::utils::{CachePadded, UnsafeDeref};

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

    pub fn reset(&self) -> bool {
        match self.state.swap(WAITING, Ordering::Acquire) {
            state if state & WOKE == WOKE => true,
            _ => false,
        }
    }

    pub unsafe fn register(&self, waker: &Waker) -> Status {
        // fast path: the correct waker is already registered
        unsafe {
            if self
                .waker
                .get()
                .deref()
                .as_ref()
                .map(|w| w.will_wake(waker))
                .unwrap_or(false)
            {
                match self.state.swap(WAITING, Ordering::Acquire) {
                    state if state & WOKE == WOKE => return Status::Retry,
                    _ => return Status::Registered,
                }
            }
        }

        // otherwise we have to register the new waker
        let mut state = self.state.load(Ordering::Relaxed);

        loop {
            // try to acquire the registration lock
            match self.state.compare_exchange_weak(
                state,
                state | REGISTERING,
                Ordering::Acquire,
                Ordering::Acquire,
            ) {
                // someone is waking the current task
                Ok(state) | Err(state) if state & WAKING == WAKING => return Status::Retry,
                // acquired the lock
                Ok(_) => break,
                // spurious failure, retry
                Err(found) => {
                    state = found;
                    continue;
                }
            }
        }

        // safety: we have the lock
        unsafe { *self.waker.get() = Some(waker.clone()) }

        // release the registration lock and reset the waker
        match self.state.swap(WAITING, Ordering::AcqRel) {
            state if state & WOKE == WOKE => Status::Retry,
            _ => Status::Registered,
        }
    }

    pub fn wake(&self) -> bool {
        let mut state = self.state.load(Ordering::Relaxed);

        loop {
            let update = if state & WOKE == WOKE {
                state
            } else {
                state | WAKING | WOKE
            };

            match self.state.compare_exchange_weak(
                state,
                update,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                // acquired the lock
                Ok(WAITING) => {
                    let waker = unsafe { self.waker.get().deref().clone() };

                    // release the lock
                    self.state.fetch_and(!WAKING, Ordering::Release);

                    if let Some(ref waker) = waker {
                        waker.wake_by_ref();
                    }

                    return true;
                }
                // succeeded but either:
                // - the current waker is already woken
                // - someone else is waking the cell
                // - a new waker is being registered; they will see our update
                Ok(_) => return false,
                // someone else woke
                Err(found) if found & (WOKE | WAKING) != 0 => {
                    return false;
                }
                // a new waker is being registered (or spurious failure), retry
                Err(found) => {
                    state = found;
                    continue;
                }
            }
        }
    }
}

pub enum Status {
    Registered,
    Retry,
}
