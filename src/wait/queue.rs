use super::Status;
use crate::util::intrusive_list::{LinkedList, Node};
use crate::util::UnsafeDeref;

use std::cell::{Cell, UnsafeCell};
use std::future::Future;
use std::hint;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::atomic::{AtomicU8, Ordering};
use std::task::{Context, Poll, Waker};

use usync::Mutex;

pub struct Queue {
    state: AtomicU8,
    list: Mutex<LinkedList<Waiter>>,
}

pub struct Waiter {
    state: AtomicU8,
    waker: Cell<Option<Waker>>,
}

impl Waiter {
    pub fn new() -> Node<Waiter> {
        Node::new(Waiter {
            state: AtomicU8::new(EMPTY),
            waker: Cell::new(None),
        })
    }
}

const EMPTY: u8 = 0;
const WAITING: u8 = 1;
const WOKE: u8 = 2;

impl Queue {
    pub fn new() -> Queue {
        Queue {
            state: AtomicU8::new(EMPTY),
            list: Mutex::new(LinkedList::new()),
        }
    }

    #[inline]
    pub async fn poll_fn<T, F>(&self, poll_fn: F) -> T
    where
        F: FnMut() -> Poll<T>,
    {
        let waiter = Node::new(Waiter {
            state: AtomicU8::new(EMPTY),
            waker: Cell::new(None),
        });

        PollFn {
            poll_fn,
            waiter,
            queue: self,
            state: State::Started,
            _value: PhantomData,
        }
        .await
    }

    pub fn register(&self, node: Pin<&mut Node<Waiter>>, waker: &Waker) -> Status {
        if self.state.load(Ordering::Relaxed) == WOKE {
            if self
                .state
                .compare_exchange(WOKE, EMPTY, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
            {
                return Status::Awoke;
            }
        }

        let mut list = self.list.lock();

        let mut state = self.state.load(Ordering::Acquire);

        loop {
            match state {
                EMPTY => match self.state.compare_exchange_weak(
                    EMPTY,
                    WAITING,
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => break,
                    Err(found) => state = found,
                },
                WOKE => match self.state.compare_exchange_weak(
                    WOKE,
                    EMPTY,
                    Ordering::Acquire,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => return Status::Awoke,
                    Err(found) => state = found,
                },
                WAITING => break,
                _ => unreachable!(),
            }
        }

        unsafe {
            node.data.waker.set(Some(waker.clone()));
            node.data.state.store(WAITING, Ordering::Release);
            list.add_front(node);
        }

        Status::Registered
    }

    pub fn poll(&self, node: Pin<&mut Node<Waiter>>, waker: &Waker) -> Poll<()> {
        if node.data.state.load(Ordering::Acquire) == WOKE {
            return Poll::Ready(());
        }

        let mut list = self.list.lock();

        if node.data.state.load(Ordering::Acquire) == WOKE {
            return Poll::Ready(());
        }

        unsafe {
            let current = node.data.waker.as_ptr().deref_mut().as_mut().unwrap();
            if !current.will_wake(waker) {
                *current = waker.clone();
            }
        }

        Poll::Pending
    }

    pub fn wake(&self) -> bool {
        let mut state = self.state.load(Ordering::Acquire);

        while matches!(state, WOKE | EMPTY) {
            match self.state.compare_exchange_weak(
                state,
                WOKE,
                Ordering::Release,
                Ordering::Relaxed,
            ) {
                Ok(_) => return false,
                Err(found) => state = found,
            }
        }

        let mut list = self.list.lock();
        let mut state = self.state.load(Ordering::Acquire);

        match state {
            EMPTY | WOKE => {
                if self
                    .state
                    .compare_exchange(state, WOKE, Ordering::Release, Ordering::Relaxed)
                    .is_err()
                {
                    self.state.store(WOKE, Ordering::Release);
                }
            }
            WAITING => {
                let waker = unsafe {
                    let waiter = list.pop().unwrap();
                    waiter.data.state.swap(WOKE, Ordering::Release);
                    waiter.data.waker.take().unwrap()
                };

                if list.is_empty() {
                    self.state.store(EMPTY, Ordering::Release);
                }

                drop(list);

                waker.wake();
            }
            _ => {}
        }

        false
    }

    pub fn wake_all(&self) {
        self.state.swap(WOKE, Ordering::Relaxed);

        let mut list = self.list.lock();
        list.drain(|waiter| {
            waiter.data.state.swap(WOKE, Ordering::Relaxed);
            if let Some(waker) = waiter.data.waker.take() {
                waker.wake();
            }
        });
    }

    pub fn remove(&self, waiter: Pin<&mut Node<Waiter>>) {
        if waiter.data.state.load(Ordering::Relaxed) != WAITING {
            return;
        }

        let mut list = self.list.lock();

        unsafe {
            list.remove(waiter);
        }

        if list.is_empty() {
            let _ =
                self.state
                    .compare_exchange(WAITING, EMPTY, Ordering::Release, Ordering::Relaxed);
        }
    }
}

pin_project_lite::pin_project! {
    struct PollFn<'a, F, T> {
        poll_fn: F,
        state: State,
        #[pin]
        waiter: Node<Waiter>,
        queue: &'a Queue,
        _value: PhantomData<T>
    }

    impl<F, T> PinnedDrop for PollFn<'_, F, T> {
        fn drop(mut this: Pin<&mut Self>) {
            let this = this.project();
            if *this.state == State::Registered {
                unsafe { this.queue.remove(this.waiter) }
            }
        }
    }
}

#[derive(PartialEq)]
enum State {
    Started,
    Registered,
    Done,
}

impl<F, T> Future for PollFn<'_, F, T>
where
    F: FnMut() -> Poll<T>,
{
    type Output = T;

    #[inline]
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        loop {
            let this = self.as_mut().project();

            match this.state {
                State::Started => {
                    match (this.poll_fn)() {
                        Poll::Ready(value) => return Poll::Ready(value),
                        Poll::Pending => {}
                    }

                    match this.queue.register(this.waiter, cx.waker()) {
                        Status::Registered => {
                            *this.state = State::Registered;
                        }
                        Status::Awoke => {
                            hint::spin_loop();
                            continue;
                        }
                    }
                }
                State::Registered => {
                    if this.queue.poll(this.waiter, cx.waker()).is_pending() {
                        return Poll::Pending;
                    }

                    *this.state = State::Done;
                }
                State::Done => match (this.poll_fn)() {
                    Poll::Ready(value) => {
                        return Poll::Ready(value);
                    }
                    Poll::Pending => match this.queue.register(this.waiter, cx.waker()) {
                        Status::Registered => {
                            *this.state = State::Registered;
                        }
                        Status::Awoke => {
                            hint::spin_loop();
                            continue;
                        }
                    },
                },
            }
        }
    }
}
