use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
use std::thread::{self, Thread};
use std::time::{Duration, Instant};

/// Run a future to completion on the current thread.
///
/// # Safety
///
/// Keeping any references to the provided waker alive after the call to
/// `block_on` is *undefined behavior*.
pub unsafe fn block_on<F: Future>(mut fut: F) -> F::Output {
    let mut fut = Pin::new_unchecked(&mut fut);

    let parker = Parker {
        thread: thread::current(),
        notified: AtomicBool::default(),
    };

    let ptr = (&parker as *const Parker).cast::<()>();
    let waker = Waker::from_raw(RawWaker::new(ptr, &VTABLE));
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

/// Run a future to completion on the current thread with a timeout.
///
/// Returns `None` if the timeout elapsed before the future resolved.
///
/// # Safety
///
/// Same as `block_on`.
pub unsafe fn block_on_timeout<F: Future>(mut fut: F, timeout: Duration) -> Option<F::Output> {
    let mut fut = Pin::new_unchecked(&mut fut);

    let parker = Parker {
        thread: thread::current(),
        notified: AtomicBool::default(),
    };

    let ptr = (&parker as *const Parker).cast::<()>();
    let waker = Waker::from_raw(RawWaker::new(ptr, &VTABLE));
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
}

struct Parker {
    thread: Thread,
    notified: AtomicBool,
}

static VTABLE: RawWakerVTable = {
    unsafe fn clone(data: *const ()) -> RawWaker {
        RawWaker::new(data, &VTABLE)
    }

    unsafe fn wake(data: *const ()) {
        let parker = &*(data.cast::<Parker>());
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
