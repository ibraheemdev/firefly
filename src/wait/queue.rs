use super::Status;
use crate::util::intrusive_list::{LinkedList, Node};
use crate::util::UnsafeDeref;

use std::cell::{Cell, UnsafeCell};
use std::pin::Pin;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Mutex;
use std::task::Waker;

pub struct Queue {
    state: AtomicU8,
    list: Mutex<LinkedList<WaiterState>>,
}

pub type Waiter = Node<WaiterState>;

pub fn waiter() -> Waiter {
    Node::new(WaiterState {
        state: AtomicU8::new(EMPTY),
        waker: Cell::new(None),
    })
}

pub struct WaiterState {
    state: AtomicU8,
    waker: Cell<Option<Waker>>,
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

    pub fn register(&self, node: Pin<&mut Waiter>, waker: &Waker) -> Status {
        if self.state.load(Ordering::Relaxed) == WOKE {
            if self
                .state
                .compare_exchange(WOKE, EMPTY, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
            {
                return Status::Awoke;
            }
        }

        let mut list = self.list.lock().unwrap();

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

    pub fn verify(&self, node: Pin<&mut Waiter>, waker: &Waker) -> bool {
        if self.state.load(Ordering::Acquire) == WOKE {
            return true;
        }

        let mut list = self.list.lock().unwrap();

        if node.data.state.load(Ordering::Acquire) == WOKE {
            return true;
        }

        unsafe {
            let current = node.data.waker.as_ptr().deref_mut().as_mut().unwrap();
            if !current.will_wake(waker) {
                *current = waker.clone();
            }
        }

        false
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

        let mut list = self.list.lock().unwrap();

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

    pub fn remove(&self, waiter: Pin<&mut Waiter>) {
        if waiter.data.state.load(Ordering::Relaxed) != WAITING {
            return;
        }

        let mut list = self.list.lock().unwrap();

        unsafe {
            list.remove(waiter);
        }

        if list.is_empty() {
            let _ =
                self.state
                    .compare_exchange(WAITING, EMPTY, Ordering::Release, Ordering::Relaxed);
        }
    }

    pub fn wake_all(&self) {
        self.state.swap(WOKE, Ordering::Relaxed);

        let mut list = self.list.lock().unwrap();
        list.drain(|waiter| {
            waiter.data.state.swap(WOKE, Ordering::Relaxed);
            if let Some(waker) = waiter.data.waker.take() {
                waker.wake();
            }
        });
    }
}
