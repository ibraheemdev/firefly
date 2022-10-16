use crate::raw::parking;
use crate::raw::util::{assert_valid_capacity, CachePadded};

use std::cell::{Cell, UnsafeCell};
use std::mem::MaybeUninit;
use std::sync::atomic::{AtomicUsize, Ordering};

// A basic ring buffer
pub struct Queue<T> {
    head: CachePadded<AtomicUsize>,
    tail: CachePadded<AtomicUsize>,
    slots: Box<[UnsafeCell<MaybeUninit<T>>]>,
}

unsafe impl<T: Send> Send for Queue<T> {}

impl<T> Queue<T> {
    pub fn new(capacity: usize) -> Self {
        assert_valid_capacity(capacity);

        Self {
            head: CachePadded(AtomicUsize::new(0)),
            tail: CachePadded(AtomicUsize::new(0)),
            // need one extra slot
            slots: (0..capacity + 1)
                .map(|_| UnsafeCell::new(MaybeUninit::uninit()))
                .collect(),
        }
    }

    #[inline]
    fn next(&self, mut pos: usize) -> usize {
        pos += 1;
        if pos == self.slots.len() {
            0
        } else {
            pos
        }
    }
}

// A local, cached copy of the queue's head/tail
pub struct Handle {
    head: Cell<usize>,
    tail: Cell<usize>,
}

impl Handle {
    pub fn new() -> Handle {
        Handle {
            head: Cell::new(0),
            tail: Cell::new(0),
        }
    }

    pub unsafe fn push<T>(&self, value: T, queue: &Queue<T>) -> Result<(), T> {
        let tail = self.tail.get();
        let head = self.head.get();

        let next_tail = queue.next(tail);

        // is the queue full according to our cached head?
        if next_tail == head {
            // load the current head
            let head = queue.head.load(Ordering::Acquire);

            // update our cache
            self.head.set(head);

            // is the queue really full?
            if next_tail == head {
                return Err(value);
            }
        }

        // store the element
        queue
            .slots
            .get_unchecked(tail)
            .get()
            .write(MaybeUninit::new(value));

        // update the tail
        queue.tail.store(next_tail, parking::RELEASE);
        self.tail.set(next_tail);

        Ok(())
    }

    pub unsafe fn pop<T>(&self, queue: &Queue<T>) -> Option<T> {
        let head = self.head.get();
        let tail = self.tail.get();

        // is the queue empty according to our cached tail?
        if head == tail {
            // load the current tail
            let tail = queue.tail.load(Ordering::Acquire);

            // update our cache
            self.tail.set(tail);

            // is the queue really empty?
            if tail == head {
                return None;
            }
        }

        // read the value
        let value = queue.slots.get_unchecked(head).get().read().assume_init();

        // update the head
        let head = queue.next(head);
        queue.head.store(head, parking::RELEASE);
        self.head.set(head);

        Some(value)
    }
}

impl<T> Drop for Queue<T> {
    fn drop(&mut self) {
        let handle = Handle::new();
        while let Some(value) = unsafe { handle.pop(&self) } {
            drop(value);
        }
    }
}
