use crate::raw::parking;
use crate::raw::util::{self, CachePadded, UnsafeDeref};

use std::cell::{Cell, UnsafeCell};
use std::mem::MaybeUninit;
use std::ptr::{self, NonNull};
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};

pub struct Queue<T> {
    head: CachePadded<Head<T>>,
    tail: CachePadded<Tail<T>>,
}

struct Head<T> {
    block: AtomicPtr<Block<T>>,
    index: Cell<usize>,
}

struct Tail<T> {
    block: Cell<*mut Block<T>>,
    index: Cell<usize>,
}

impl<T> Queue<T> {
    pub fn new() -> Self {
        Self {
            tail: CachePadded(Tail {
                block: Cell::new(ptr::null_mut()),
                index: Cell::new(0),
            }),
            head: CachePadded(Head {
                block: AtomicPtr::new(ptr::null_mut()),
                index: Cell::new(0),
            }),
        }
    }

    pub unsafe fn push(&self, value: T) {
        let index = self.tail.index.get();
        let next_index = (index + 1) % SLOTS;
        let mut block = self.tail.block.get();

        if block.is_null() || next_index == 0 {
            let new_block = Block::alloc();

            if block.is_null() {
                block = new_block;
                self.head.block.store(new_block, parking::RELEASE);
            } else {
                block.deref().next.store(new_block, parking::RELEASE);
            }

            self.tail.block.set(new_block);
        }

        self.tail.index.set(next_index);
        (*block).slots.get_unchecked(index).store(value);
    }

    pub unsafe fn pop(&self) -> Option<T> {
        let block = self.head.block.load(Ordering::Acquire);
        let mut block = NonNull::new(block)?;

        let mut index = self.head.index.get();
        if index == SLOTS {
            let next_block = block.as_ref().next.load(Ordering::Acquire);
            let next_block = NonNull::new(next_block)?;

            drop(Box::from_raw(block.as_ptr()));
            block = next_block;
            index = 0;

            self.head.block.store(block.as_ptr(), Ordering::Relaxed);
            self.head.index.set(index);
        }

        let value = block.as_ref().slots.get_unchecked(index).load()?;
        self.head.index.set(index + 1);
        Some(value)
    }
}

impl<T> Drop for Queue<T> {
    fn drop(&mut self) {
        unsafe {
            while let Some(value) = self.pop() {
                drop(value);
            }

            let block = self.head.block.load(Ordering::Relaxed);
            if !block.is_null() {
                drop(Box::from_raw(block));
            }
        }
    }
}

const SLOTS: usize = 1024;

struct Block<T> {
    slots: [Slot<T>; SLOTS],
    next: AtomicPtr<Block<T>>,
}

impl<T> Block<T> {
    fn alloc() -> *mut Block<T> {
        unsafe { Box::into_raw(util::box_zeroed()) }
    }
}

struct Slot<T> {
    value: UnsafeCell<MaybeUninit<T>>,
    active: AtomicBool,
}

impl<T> Slot<T> {
    unsafe fn store(&self, value: T) {
        self.value.get().write(MaybeUninit::new(value));
        self.active.store(true, parking::RELEASE);
    }

    unsafe fn load(&self) -> Option<T> {
        if self.active.load(Ordering::Acquire) {
            return Some(self.value.get().read().assume_init());
        }

        None
    }
}
