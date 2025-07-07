use std::cell::UnsafeCell;
use std::sync::atomic::{self, AtomicU8, Ordering};
use std::task::{self, Poll, Waker};

// No active notifications.
const IDLE: u8 = 0;

// The task was notified.
const NOTIFIED: u8 = 0b001;

// A write-lock for registering a waker.
const REGISTERING: u8 = 0b010;

// A read-lock for waking a task.
const WAKING: u8 = 0b100;

/// A multi-producer single-consumer task wakeup primitive.
#[derive(Debug)]
pub struct AtomicWaker {
    state: AtomicU8,
    waker: UnsafeCell<Waker>,
}

impl AtomicWaker {
    /// Creates a new `AtomicWaker`.
    pub fn new() -> Self {
        Self {
            state: AtomicU8::new(IDLE),
            waker: UnsafeCell::new(task::Waker::noop().clone()),
        }
    }

    /// Sends a notification if a waker is registered.
    pub fn notify(&self) {
        atomic::fence(Ordering::SeqCst);
        self.notify_seqcst();
    }

    /// Sends a notification if a waker is registered.
    pub fn notify_seqcst(&self) {
        let mut state = self.state.load(Ordering::SeqCst);

        loop {
            // The notification token is already set, we're done. The future consumer
            // will see the token and retry.
            if state & NOTIFIED != 0 {
                return;
            }

            // Otherwise, we have to set the notification.
            let mut new = state | NOTIFIED;

            // If the consumer is currently registering a new waker, they will see
            // our token once they release the lock.
            if state & REGISTERING == 0 {
                // Otherwise, we have to acquire the waking lock.
                new |= WAKING;
            }

            match self
                .state
                .compare_exchange_weak(state, new, Ordering::AcqRel, Ordering::Relaxed)
            {
                // We set the token and acquired the lock, now we have to wake the task.
                Ok(IDLE) => {
                    // SAFETY: We hold the `WAKING` lock.
                    unsafe { (*self.waker.get()).wake_by_ref() };

                    // Release the lock.
                    self.state.fetch_sub(WAKING, Ordering::Release);

                    return;
                }

                // The task is still registering it's waker, they will see our token.
                // It is also possible that someone else beat us and acquired the waking
                // lock.
                Ok(_) => return,

                // Something changed, retry.
                Err(found) => state = found,
            }
        }
    }

    /// Registers a new waker.
    pub unsafe fn register(&self, waker: &Waker) -> Poll<()> {
        let result = unsafe { self.register_seqcst(waker) };
        atomic::fence(Ordering::SeqCst);
        result
    }

    /// Registers a new waker.
    pub unsafe fn register_seqcst(&self, waker: &Waker) -> Poll<()> {
        let mut state = self.state.load(Ordering::Acquire);

        // If a notification is not set, we have to register our waker.
        if state == IDLE {
            // Safety: We are the sole consumer, so it is always safe to
            // dereference the waker. Producers only have read-only access
            // while holding the waking lock.
            let will_wake = unsafe { (*self.waker.get()).will_wake(waker) };

            // Fast-path: The correct waker is already registered.
            if will_wake {
                return Poll::Pending;
            }

            // Otherwise, we have to register our waker.
            match unsafe { self.register_slow(waker) } {
                Ok(status) => return status,
                Err(found) => state = found,
            }
        }

        // If a notification is set and the waking lock is currently held,
        // try to eagerly consume the token instead of wasting a cycle next
        // time.
        if state == (WAKING | NOTIFIED) {
            match self
                .state
                .compare_exchange(state, WAKING, Ordering::SeqCst, Ordering::Acquire)
            {
                // We consumed the token, so were woken.
                Ok(_) => return Poll::Ready(()),

                // The task released the lock.
                Err(found) => state = found,
            }
        }

        // If a notification is set but the waking lock is not held, we can
        // simply consume the token.
        if state == NOTIFIED {
            // Note that the waking lock is never acquired if a notification
            // is already set, so there's no risk of overwriting anything here.
            self.state.store(IDLE, Ordering::SeqCst);

            // We consumed the token, so were woken.
            return Poll::Ready(());
        }

        // If the waking lock is still held but we already consumed the notification, we
        // have to retry.
        Poll::Ready(())
    }

    #[cold]
    #[inline(never)]
    unsafe fn register_slow(&self, waker: &Waker) -> Result<Poll<()>, u8> {
        // Try to acquire the registration lock.
        self.state
            .compare_exchange(IDLE, REGISTERING, Ordering::Acquire, Ordering::Acquire)?;

        // Store our waker.
        //
        // Safety: We hold the lock.
        unsafe { *self.waker.get() = waker.clone() };

        // Release the lock.
        match self.state.swap(IDLE, Ordering::SeqCst) {
            // Nothing happened in the meantime, we succesfully registered.
            REGISTERING => return Ok(Poll::Pending),

            // Someone tried to wake us in the meantime.
            _ => return Ok(Poll::Ready(())),
        }
    }
}
