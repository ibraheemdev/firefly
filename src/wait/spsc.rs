use super::will_wake;
use crate::util::UnsafeDeref;

use std::cell::UnsafeCell;
use std::hint;
use std::sync::atomic::fence;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::task::Context;
use std::task::Poll;
use std::task::Waker;

pub struct WaitCell {
    waker: UnsafeCell<Option<Waker>>,
    locked: AtomicBool,
    woke: AtomicBool,
}

unsafe impl Send for WaitCell {}
unsafe impl Sync for WaitCell {}

impl WaitCell {
    pub fn new() -> WaitCell {
        WaitCell {
            waker: UnsafeCell::new(None),
            locked: AtomicBool::new(false),
            woke: AtomicBool::new(true),
        }
    }

    #[inline]
    pub fn poll_with<T, F>(&self, cx: &mut Context<'_>, mut poll: F) -> Poll<T>
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

    unsafe fn poll(&self, waker: &Waker) -> Poll<()> {
        if !self.woke.load(Ordering::Acquire) {
            let locked = !self.locked.swap(true, Ordering::Acquire);
            if locked {
                let will_wake = will_wake(self.waker.get().deref(), waker);
                if !will_wake {
                    *self.waker.get() = Some(waker.clone());
                }

                self.locked.store(false, Ordering::SeqCst);
            }

            if !self.woke.load(Ordering::SeqCst) {
                assert!(locked);
                return Poll::Pending;
            }
        }

        self.woke.store(false, Ordering::Relaxed);
        Poll::Ready(())
    }

    pub fn wake(&self) {
        fence(Ordering::SeqCst);

        if self.woke.load(Ordering::Relaxed) {
            return;
        }

        self.woke.store(true, Ordering::Release);
        if self.locked.swap(true, Ordering::AcqRel) {
            return;
        }

        let waker = unsafe { self.waker.get().deref_mut().take() };
        self.locked.store(false, Ordering::Release);

        if let Some(waker) = waker {
            waker.wake();
        }
    }
}
