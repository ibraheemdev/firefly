use crate::raw::parking;
use crate::raw::util::CachePadded;

use std::cell::{Cell, UnsafeCell};
use std::mem::MaybeUninit;
use std::sync::atomic::{AtomicUsize, Ordering};

pub struct Queue<T> {
    head: CachePadded<AtomicUsize>,
    tail: CachePadded<AtomicUsize>,
    slots: Box<[UnsafeCell<MaybeUninit<T>>]>,
}

unsafe impl<T: Send> Send for Queue<T> {}

impl<T> Queue<T> {
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.next_power_of_two();

        Self {
            head: CachePadded(AtomicUsize::new(0)),
            tail: CachePadded(AtomicUsize::new(0)),
            slots: (0..capacity)
                .map(|_| UnsafeCell::new(MaybeUninit::uninit()))
                .collect(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.head.load(Ordering::Relaxed) == self.tail.load(Ordering::Relaxed)
    }

    #[inline]
    fn next(&self, pos: usize) -> usize {
        let pos = pos + 1;
        if pos == self.slots.len() {
            0
        } else {
            pos
        }
    }
}

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

        if next_tail == head {
            let head = queue.head.load(Ordering::Acquire);
            self.head.set(head);
            if next_tail == head {
                return Err(value);
            }
        }

        queue
            .slots
            .get_unchecked(tail)
            .get()
            .write(MaybeUninit::new(value));

        queue.tail.store(next_tail, parking::RELEASE);
        self.tail.set(next_tail);

        Ok(())
    }

    pub unsafe fn pop<T>(&self, queue: &Queue<T>) -> Option<T> {
        let head = self.head.get();
        let tail = self.tail.get();

        if head == tail {
            let tail = queue.tail.load(Ordering::Acquire);
            self.tail.set(tail);
            if tail == head {
                return None;
            }
        }

        let value = queue.slots.get_unchecked(head).get().read().assume_init();

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
