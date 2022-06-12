use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
use std::thread::{self, Thread};

pub unsafe fn block_on<F>(mut future: F) -> F::Output
where
    F: Future,
{
    let mut future = Pin::new_unchecked(&mut future);

    PARKER.with(|parker| {
        let data = parker as *const Parker as *const ();
        let waker = Waker::from_raw(RawWaker::new(data, &VTABLE));
        let mut cx = Context::from_waker(&waker);

        loop {
            if let Poll::Ready(value) = future.as_mut().poll(&mut cx) {
                return value;
            }

            while !parker.notified.swap(false, Ordering::Acquire) {
                thread::park();
            }
        }
    })
}

struct Parker {
    thread: Thread,
    notified: AtomicBool,
}

thread_local! {
    static PARKER: Parker = Parker {
        thread: thread::current(),
        notified: AtomicBool::new(false)
    };
}

static VTABLE: RawWakerVTable = {
    unsafe fn clone(data: *const ()) -> RawWaker {
        RawWaker::new(data, &VTABLE)
    }

    unsafe fn wake(data: *const ()) {
        let parker = &*(data as *const Parker);
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
