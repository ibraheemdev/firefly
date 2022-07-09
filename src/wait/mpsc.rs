use super::will_wake;
use crate::util::UnsafeDeref;

use std::cell::UnsafeCell;
use std::hint;
use std::sync::atomic::fence;
use std::sync::atomic::AtomicU8;
use std::sync::atomic::Ordering;
use std::task::Context;
use std::task::Poll;
use std::task::Waker;

#[derive(Default)]
pub struct WaitCell {
    state: AtomicU8,
    waker: UnsafeCell<Option<Waker>>,
}

unsafe impl Send for WaitCell {}
unsafe impl Sync for WaitCell {}

const WAITING: u8 = 0b000;
const WOKE: u8 = 0b001;
const REGISTERING: u8 = 0b010;
const WAKING: u8 = 0b100;

impl WaitCell {
    pub fn new() -> WaitCell {
        WaitCell {
            state: AtomicU8::new(WAITING),
            waker: UnsafeCell::new(None),
        }
    }

    #[inline]
    pub fn poll_fn<T, F>(&self, cx: &mut Context<'_>, mut poll: F) -> Poll<T>
    where
        F: FnMut() -> Poll<T>,
    {
        loop {
            match (poll)() {
                Poll::Ready(value) => return Poll::Ready(value),
                Poll::Pending => {}
            }

            match unsafe { self.poll(cx.waker()) } {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(_) => hint::spin_loop(),
            }
        }
    }

    pub unsafe fn poll(&self, waker: &Waker) -> Poll<()> {
        let mut state = self.state.load(Ordering::Relaxed);

        if state == WAITING {
            let will_wake = will_wake(self.waker.get().deref(), waker);

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

    pub fn wake(&self) {
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
