use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
use std::thread::{self, Thread};
use std::time::{Duration, Instant};

/// Run a future to completion on the current thread.
pub fn block_on<F: Future>(mut fut: F) -> F::Output {
    let mut fut = unsafe { Pin::new_unchecked(&mut fut) };

    let parker = Parker {
        thread: thread::current(),
        notified: AtomicBool::default(),
    };

    let ptr = (&parker as *const Parker).cast::<()>();
    let waker = unsafe { Waker::from_raw(RawWaker::new(ptr, &VTABLE)) };
    let mut cx = Context::from_waker(&waker);

    loop {
        if let Poll::Ready(output) = fut.as_mut().poll(&mut cx) {
            return output;
        }

        while !parker.notified.swap(false, Ordering::Acquire) {
            thread::park();
        }
    }
}

macro_rules! poll_fn {
    (|$cx:ident| $f:expr) => {{
        use ::std::sync::{atomic::Ordering, Arc};
        use ::std::task::{Context, RawWaker, Waker};
        use ::std::thread;
        use $crate::raw::blocking::VTABLE;

        $crate::raw::blocking::PARKER.with(|parker| unsafe {
            let ptr = Arc::into_raw(parker.clone()).cast::<()>();
            let waker = Waker::from_raw(RawWaker::new(ptr, &VTABLE));
            let $cx = Context::from_waker(&waker);

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
}

macro_rules! poll_fn_timeout {
    ($timeout:expr, |$cx:ident| $f:expr) => {{
        use ::std::sync::{atomic::Ordering, Arc};
        use ::std::task::{Context, RawWaker, Waker};
        use ::std::thread;
        use ::std::time::Instant;
        use $crate::raw::blocking::VTABLE;

        $crate::raw::blocking::PARKER.with(|parker| unsafe {
            let ptr = Arc::into_raw(parker.clone()).cast::<()>();
            let waker = Waker::from_raw(RawWaker::new(ptr, &VTABLE));
            let $cx = Context::from_waker(&waker);

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

pub(crate) use {poll_fn, poll_fn_timeout};

/// Run a future to completion on the current thread with a timeout.
///
/// Returns `None` if the timeout elapsed before the future resolved.
pub fn block_on_timeout<F: Future>(mut fut: F, timeout: Duration) -> Option<F::Output> {
    let mut fut = unsafe { Pin::new_unchecked(&mut fut) };

    PARKER.with(|parker| {
        let ptr = Arc::into_raw(parker.clone()).cast::<()>();
        let waker = unsafe { Waker::from_raw(RawWaker::new(ptr, &VTABLE)) };
        let mut cx = Context::from_waker(&waker);

        let start = Instant::now();
        let mut remaining = timeout;

        loop {
            if let Poll::Ready(value) = fut.as_mut().poll(&mut cx) {
                return Some(value);
            }

            while !parker.notified.swap(false, Ordering::Acquire) {
                thread::park_timeout(remaining);

                let elapsed = start.elapsed();
                if elapsed >= timeout {
                    return None;
                }

                remaining = timeout - elapsed;
            }
        }
    })
}

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

pub static VTABLE: RawWakerVTable = {
    unsafe fn clone(data: *const ()) -> RawWaker {
        RawWaker::new(data, &VTABLE)
    }

    unsafe fn wake(data: *const ()) {
        let parker = Arc::from_raw(data as *const Parker);
        if !parker.notified.swap(true, Ordering::Release) {
            parker.thread.unpark();
        }
    }

    unsafe fn wake_by_ref(data: *const ()) {
        wake(data)
    }

    unsafe fn destroy(_: *const ()) {}

    RawWakerVTable::new(clone, wake, wake_by_ref, destroy)
};
