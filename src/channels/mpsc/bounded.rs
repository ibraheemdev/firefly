use crate::raw::parking;
use crate::raw::util::{assert_valid_capacity, CachePadded};

use std::cell::{Cell, UnsafeCell};
use std::mem::{drop, MaybeUninit};
use std::sync::atomic::{AtomicBool, Ordering};

pub struct Queue<T> {
    sema: CachePadded<atomic::Semaphore>,
    tail: CachePadded<atomic::Counter>,
    head: CachePadded<Cell<usize>>,
    slots: Box<[Slot<T>]>,
}

struct Slot<T> {
    stored: AtomicBool,
    value: UnsafeCell<MaybeUninit<T>>,
}

unsafe impl<T: Send> Send for Queue<T> {}

impl<T> Queue<T> {
    pub fn new(capacity: usize) -> Self {
        assert_valid_capacity(capacity);

        Self {
            sema: CachePadded(atomic::Semaphore::new(capacity)),
            tail: CachePadded(atomic::Counter::default()),
            head: CachePadded(Cell::new(0)),
            slots: (0..capacity)
                .map(|_| Slot {
                    stored: AtomicBool::new(false),
                    value: UnsafeCell::new(MaybeUninit::uninit()),
                })
                .collect(),
        }
    }

    pub fn push(&self, value: T) -> Result<(), T> {
        if !self.sema.try_acquire() {
            return Err(value);
        }

        let pos = self.tail.fetch_inc();
        let index = pos & (self.slots.len() - 1);

        unsafe {
            let slot = self.slots.get_unchecked(index);
            slot.value.get().write(MaybeUninit::new(value));
            slot.stored.store(true, parking::RELEASE);
        }

        Ok(())
    }

    pub fn can_push(&self) -> bool {
        self.sema.can_push()
    }

    pub unsafe fn is_empty(&self) -> bool {
        self.head.get() == self.tail.load()
    }

    pub unsafe fn pop(&self) -> Option<T> {
        let pos = self.head.get();
        let index = pos & (self.slots.len() - 1);

        let slot = self.slots.get_unchecked(index);
        if !slot.stored.load(Ordering::Acquire) {
            return None;
        }

        let value = slot.value.get().read().assume_init();
        slot.stored.store(false, Ordering::Release);

        self.sema.release();

        self.head.set(pos.wrapping_add(1));
        Some(value)
    }
}

impl<T> Drop for Queue<T> {
    fn drop(&mut self) {
        while let Some(value) = unsafe { self.pop() } {
            drop(value);
        }
    }
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
mod atomic {
    use super::*;

    use std::hint;
    use std::sync::atomic::{AtomicIsize, AtomicUsize, Ordering};

    pub struct Semaphore(AtomicIsize);

    impl Semaphore {
        pub fn new(permits: usize) -> Self {
            Self(AtomicIsize::new(permits.try_into().unwrap()))
        }

        pub fn try_acquire(&self) -> bool {
            let mut spun = 0_usize;

            loop {
                if self.0.fetch_sub(1, Ordering::Acquire) > 0 {
                    return true;
                }

                spun += 1;
                for _ in 0..spun.pow(2) {
                    hint::spin_loop();
                }

                if self.0.fetch_add(1, Ordering::Relaxed) < 0 {
                    return false;
                }

                spun += 1;
                for _ in 0..spun.pow(2) {
                    hint::spin_loop();
                }
            }
        }

        pub fn can_push(&self) -> bool {
            self.0.load(Ordering::Relaxed) > 0
        }

        pub fn release(&self) {
            self.0.fetch_add(1, parking::RELEASE);
        }
    }

    #[derive(Default)]
    pub struct Counter(AtomicUsize);

    impl Counter {
        pub fn fetch_inc(&self) -> usize {
            self.0.fetch_add(1, Ordering::Relaxed)
        }

        pub fn load(&self) -> usize {
            self.0.load(Ordering::Relaxed)
        }
    }
}

#[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
mod atomic {
    use super::*;

    use std::sync::atomic::{AtomicUsize, Ordering};

    pub struct Semaphore(AtomicUsize);

    impl Semaphore {
        pub fn new(permits: usize) -> Self {
            Self(AtomicUsize::new(permits))
        }

        pub fn try_acquire(&self) -> bool {
            fetch_update(&self.0, Ordering::Acquire, |v| v.checked_sub(1)).is_ok()
        }

        pub fn release(&self) {
            let _ = fetch_update(&self.0, parking::RELEASE, |v| Some(v + 1)).unwrap();
        }
    }

    #[derive(Default)]
    pub struct Counter(AtomicUsize);

    impl Counter {
        pub fn fetch_inc(&self) -> usize {
            fetch_update(&self.0, Ordering::Relaxed, |v| Some(v + 1)).unwrap()
        }

        pub fn load(&self) -> usize {
            self.0.load(Ordering::Relaxed)
        }
    }

    #[inline(always)]
    fn fetch_update(
        value: &AtomicUsize,
        success: Ordering,
        mut update: impl FnMut(usize) -> Option<usize>,
    ) -> Result<usize, usize> {
        loop {
            let v = value.load(Ordering::Relaxed);
            let new_v = update(v).ok_or(v)?;
            match value.compare_exchange(v, new_v, success, Ordering::Relaxed) {
                Ok(_) => return Ok(v),
                Err(_) => std::thread::yield_now(),
            }
        }
    }
}
