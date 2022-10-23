use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::task::{Poll, Wake};
use std::thread::{self, Thread};
use std::time::Duration;

/// Run a future to completion on the current thread.
pub fn block_on<F: Future>(mut fut: F) -> F::Output {
    let mut fut = unsafe { Pin::new_unchecked(&mut fut) };
    poll_fn!(|cx| fut.as_mut().poll(&mut cx))
}

/// Run a future to completion on the current thread with a timeout.
///
/// Returns `None` if the timeout elapsed before the future resolved.
pub fn block_on_timeout<F: Future>(mut fut: F, timeout: Duration) -> Option<F::Output> {
    let mut fut = unsafe { Pin::new_unchecked(&mut fut) };
    poll_fn!(timeout, |cx| fut.as_mut().poll(&mut cx))
}

macro_rules! poll_fn {
    (|$cx:ident| $f:expr) => {{
        use ::std::sync::atomic::Ordering;
        use ::std::task::Context;
        use ::std::thread;

        #[allow(unused)]
        $crate::raw::blocking::PARKER.with(|parker| unsafe {
            let waker = parker.clone().into();
            let mut $cx = Context::from_waker(&waker);

            loop {
                if let Poll::Ready(output) = { $f } {
                    return output;
                }

                while !parker.notified.swap(false, Ordering::Acquire) {
                    thread::park();
                }
            }
        })
    }};
    ($timeout:expr, |$cx:ident| $f:expr) => {{
        use ::std::sync::atomic::Ordering;
        use ::std::task::Context;
        use ::std::thread;
        use ::std::time::Instant;

        #[allow(unused)]
        $crate::raw::blocking::PARKER.with(|parker| unsafe {
            let waker = parker.clone().into();
            let mut $cx = Context::from_waker(&waker);

            let timeout = $timeout;
            let start = Instant::now();
            let mut remaining = $timeout;

            'poll: loop {
                if let Poll::Ready(value) = { $f } {
                    break Some(value);
                }

                while !parker.notified.swap(false, Ordering::Acquire) {
                    thread::park_timeout(remaining);

                    let elapsed = start.elapsed();
                    if elapsed >= timeout {
                        break 'poll None;
                    }

                    remaining = timeout - elapsed;
                }
            }
        })
    }};
}

pub(crate) use poll_fn;

thread_local! {
    pub static PARKER: Arc<Parker> = Arc::new(Parker {
        thread: thread::current(),
        notified: AtomicBool::default(),
    });
}

pub struct Parker {
    pub thread: Thread,
    pub notified: AtomicBool,
}

impl Wake for Parker {
    fn wake(self: Arc<Self>) {
        if !self.notified.swap(true, Ordering::Release) {
            self.thread.unpark();
        }
    }

    fn wake_by_ref(self: &Arc<Self>) {
        panic!("wake_by_ref")
    }
}
