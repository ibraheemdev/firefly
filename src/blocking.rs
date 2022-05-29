use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
use std::thread::{self, Thread};

pub unsafe fn block_on<F>(mut future: F) -> F::Output
where
    F: Future,
{
    PARKER.with(|parker| {
        let mut future = unsafe { Pin::new_unchecked(&mut future) };

        let data = parker as *const Parker as *const ();
        let waker = unsafe { Waker::from_raw(RawWaker::new(data, &VTABLE)) };
        let mut cx = Context::from_waker(&waker);

        loop {
            if let Poll::Ready(value) = future.as_mut().poll(&mut cx) {
                return value;
            }

            while parker.parked.swap(true, Ordering::Acquire) {
                thread::park();
            }
        }
    })
}

struct Parker {
    thread: Thread,
    parked: AtomicBool,
}

thread_local! {
    static PARKER: Parker = Parker {
        thread: thread::current(),
        parked: AtomicBool::new(true)
    };
}

static VTABLE: RawWakerVTable = {
    unsafe fn clone(data: *const ()) -> RawWaker {
        RawWaker::new(data, &VTABLE)
    }

    unsafe fn wake(data: *const ()) {
        wake_by_ref(data)
    }

    unsafe fn wake_by_ref(data: *const ()) {
        let parker = &*(data as *const Parker);
        let parked = parker.parked.swap(false, Ordering::Release);
        if parked {
            parker.thread.unpark();
        }
    }

    unsafe fn destroy(_: *const ()) {}

    RawWakerVTable::new(clone, wake, wake_by_ref, destroy)
};
