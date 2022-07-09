use std::{
    cell::{Cell, UnsafeCell},
    future::Future,
    marker::PhantomPinned,
    mem::drop,
    pin::Pin,
    sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering},
    task::{Context, Poll, RawWaker, RawWakerVTable, Waker},
    thread,
};
use usync::Mutex;

use crate::util::intrusive_list::{LinkedList, Node};

const EMPTY: u8 = 0;
const WAITING: u8 = 1;
const UPDATING: u8 = 2;
const NOTIFIED: u8 = 3;

#[derive(Default)]
struct AtomicWaker {
    state: AtomicU8,
    waker: UnsafeCell<Option<Waker>>,
}

impl AtomicWaker {
    fn poll(&self, waker: &Waker) -> Poll<()> {
        let state = self.state.load(Ordering::Acquire);
        if state == NOTIFIED {
            return Poll::Ready(());
        }

        assert!(state == EMPTY || state == WAITING);
        if let Err(state) =
            self.state
                .compare_exchange(state, UPDATING, Ordering::Acquire, Ordering::Acquire)
        {
            assert_eq!(state, NOTIFIED);
            return Poll::Ready(());
        }

        unsafe {
            let w = &mut *self.waker.get();
            if !w.as_ref().map(|w| w.will_wake(waker)).unwrap_or(false) {
                *w = Some(waker.clone());
            }
        }

        match self
            .state
            .compare_exchange(UPDATING, WAITING, Ordering::AcqRel, Ordering::Acquire)
        {
            Ok(_) => Poll::Pending,
            Err(NOTIFIED) => Poll::Ready(()),
            Err(_) => unreachable!(),
        }
    }

    fn wake(&self) {
        let state = self.state.swap(NOTIFIED, Ordering::AcqRel);
        if state != WAITING {
            return;
        }

        let waker = unsafe { (*self.waker.get()).take() };
        waker.map(Waker::wake).unwrap_or(())
    }
}

type Waiter = Node<WaiterData>;

#[derive(Default)]
struct WaiterData {
    waiting: Cell<bool>,
    waker: AtomicWaker,
}

pub struct WaitQueue {
    pending: AtomicUsize,
    waiters: Mutex<LinkedList<WaiterData>>,
}

impl WaitQueue {
    pub fn new() -> Self {
        Self {
            pending: 0.into(),
            waiters: Mutex::new(LinkedList::new()),
        }
    }

    unsafe fn block_on_pinned<F: Future>(mut fut: Pin<&mut F>) -> F::Output {
        struct Signal {
            thread: thread::Thread,
            notified: AtomicBool,
            _pin: PhantomPinned,
        }

        let signal = Signal {
            thread: thread::current(),
            notified: AtomicBool::default(),
            _pin: PhantomPinned,
        };
        let signal = Pin::new_unchecked(&signal);

        const VTABLE: RawWakerVTable = RawWakerVTable::new(
            |ptr| RawWaker::new(ptr, &VTABLE),
            |ptr| unsafe {
                let signal = &*ptr.cast::<Signal>();
                if !signal.notified.swap(true, Ordering::Release) {
                    signal.thread.unpark();
                }
            },
            |_ptr| unreachable!("wake_by_ref"),
            |_ptr| {},
        );

        let ptr = (&*signal as *const Signal).cast::<()>();
        let waker = Waker::from_raw(RawWaker::new(ptr, &VTABLE));
        let mut cx = Context::from_waker(&waker);

        loop {
            if let Poll::Ready(output) = fut.as_mut().poll(&mut cx) {
                return output;
            }
            while !signal.notified.swap(false, Ordering::Acquire) {
                thread::park();
            }
        }
    }

    pub async fn poll_fnn<T>(
        &self,
        mut should_park: impl FnMut() -> bool,
        mut try_poll: impl FnMut() -> Poll<T>,
    ) -> T {
        loop {
            match try_poll() {
                Poll::Ready(value) => return value,
                Poll::Pending => {
                    std::hint::spin_loop();
                    self.park(|| should_park()).await;
                }
            }
        }
    }

    pub async fn poll_fn<T>(&self, mut try_poll: impl FnMut() -> Poll<T>) -> T {
        panic!("...")
    }

    pub async fn park(&self, should_park: impl FnOnce() -> bool) {
        struct ParkFuture<'a, 'b> {
            parker: &'a WaitQueue,
            waiter: Option<Pin<&'b mut Waiter>>,
        }

        impl<'a, 'b> Future for ParkFuture<'a, 'b> {
            type Output = ();

            fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
                let waiter = self
                    .waiter
                    .take()
                    .expect("ParkFuture polled after completion");

                if waiter.data.waker.poll(cx.waker()).is_ready() {
                    return Poll::Ready(());
                }

                self.waiter = Some(waiter);
                Poll::Pending
            }
        }

        impl<'a, 'b> Drop for ParkFuture<'a, 'b> {
            fn drop(&mut self) {
                if let Some(mut waiter) = self.waiter.take() {
                    let mut lock = self.parker.waiters.lock();
                    unsafe {
                        if waiter.as_ref().data.waiting.replace(false) {
                            assert!(lock.remove(waiter.as_mut()));
                            self.parker.pending.fetch_sub(1, Ordering::Relaxed);
                            return;
                        }

                        drop(lock);

                        self.waiter = Some(waiter);
                        WaitQueue::block_on_pinned(Pin::new(self));
                    }
                }
            }
        }

        self.pending.fetch_add(1, Ordering::SeqCst);
        let mut waiters = self.waiters.lock();

        if !should_park() {
            drop(waiters);
            self.pending.fetch_sub(1, Ordering::Relaxed);
            return;
        }

        unsafe {
            let mut waiter = Waiter::new(WaiterData {
                waiting: Cell::new(true),
                waker: AtomicWaker::default(),
            });

            let mut waiter = Pin::new_unchecked(&mut waiter);

            waiters.add_front(waiter.as_mut());
            drop(waiters);

            ParkFuture {
                parker: self,
                waiter: Some(waiter),
            }
            .await;
        }
    }

    pub fn wake(&self) {
        if self.pending.load(Ordering::SeqCst) == 0 {
            return;
        }

        let mut waiters = self.waiters.lock();
        if let Some(waiter) = waiters.pop() {
            assert!(waiter.data.waiting.replace(false));

            let waker = &mut waiter.data.waker as *mut AtomicWaker;
            drop(waiters);
            self.pending.fetch_sub(1, Ordering::Relaxed);
            unsafe {
                (*waker).wake();
            }
        }
    }

    pub fn wake_all(&self) {
        if self.pending.load(Ordering::SeqCst) == 0 {
            return;
        }

        let mut waiters = self.waiters.lock();
        let mut x = 0;
        waiters.drain(|waiter| {
            assert!(waiter.data.waiting.replace(false));
            waiter.data.waker.wake();
            x += 1;
        });

        if x > 0 {
            self.pending.fetch_sub(x, Ordering::Relaxed);
        }
    }
}
