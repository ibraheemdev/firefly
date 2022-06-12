use crate::util::{CachePadded, UnsafeDeref};

use std::cell::UnsafeCell;
use std::mem::MaybeUninit;
use std::sync::atomic::{AtomicIsize, AtomicUsize, Ordering};
use std::{hint, iter, ptr};

pub struct Queue<T> {
    free: IndexQueue,
    elements: IndexQueue,
    slots: Box<[UnsafeCell<MaybeUninit<T>>]>,
    order: usize,
}

unsafe impl<T: Send> Send for Queue<T> {}
unsafe impl<T: Send> Sync for Queue<T> {}

impl<T> Queue<T> {
    pub fn new(capacity: usize) -> Self {
        if capacity == 0 {
            panic!("capacity must be non-zero");
        }

        let capacity = capacity.next_power_of_two();
        let order = (usize::BITS - capacity.leading_zeros() - 1) as _;

        if order > 31 {
            panic!("exceeded maximum queue capacity of {}", MAX_CAPACITY);
        }

        Queue {
            slots: (0..capacity)
                .map(|_| UnsafeCell::new(MaybeUninit::uninit()))
                .collect(),
            free: IndexQueue::full(order),
            elements: IndexQueue::empty(order),
            order,
        }
    }

    pub fn push(&self, value: T) -> Result<(), T> {
        match self.free.pop(self.order) {
            Some(elem) => unsafe {
                self.slots
                    .get_unchecked(elem)
                    .get()
                    .write(MaybeUninit::new(value));
                self.elements.push(elem, self.order);
                Ok(())
            },
            None => Err(value),
        }
    }

    pub fn pop(&self) -> Option<T> {
        match self.elements.pop(self.order) {
            Some(elem) => unsafe {
                let value = self.slots.get_unchecked(elem).get().read().assume_init();
                self.free.push(elem, self.order);
                Some(value)
            },
            None => None,
        }
    }

    pub fn len(&self) -> usize {
        self.elements.len(self.order)
    }

    pub fn is_empty(&self) -> bool {
        self.elements.len(self.order) == 0
    }

    pub fn is_full(&self) -> bool {
        self.free.len(self.order) == 0
    }

    pub fn capacity(&self) -> usize {
        1 << self.order
    }
}

impl<T> Drop for Queue<T> {
    fn drop(&mut self) {
        while let Some(elem) = self.elements.pop(self.order) {
            let _: T = unsafe { self.slots[elem].get().read().assume_init() };
        }
    }
}

// Note: if set by a user, this will round up to 1 << 32.
const MAX_CAPACITY: usize = (1 << 31) + 1;

const SPIN_LIMIT: usize = 6;

pub struct IndexQueue {
    head: CachePadded<AtomicUsize>,
    tail: CachePadded<AtomicUsize>,
    threshold: CachePadded<AtomicIsize>,
    slots: CachePadded<Box<[AtomicUsize]>>,
}

impl IndexQueue {
    pub fn empty(order: usize) -> IndexQueue {
        let capacity = 1 << order;

        // the number of slots is double the capacity
        // such that the last reader can always locate
        // an unused slot no farther than capacity*2 slots
        // away from the last writer
        let slots = capacity * 2;

        IndexQueue {
            head: CachePadded(AtomicUsize::new(0)),
            tail: CachePadded(AtomicUsize::new(0)),
            threshold: CachePadded(AtomicIsize::new(-1)),
            slots: CachePadded(
                iter::repeat(-1_isize as usize)
                    .map(AtomicUsize::new)
                    .take(slots)
                    .collect(),
            ),
        }
    }

    pub fn full(order: usize) -> IndexQueue {
        let capacity = 1 << order;
        let slots = capacity * 2;

        let mut queue = IndexQueue::empty(order);

        for i in 0..capacity {
            queue.slots[cache_remap!(i, order, slots)] =
                AtomicUsize::new(raw_cache_remap!(slots + i, order, capacity));
        }

        *queue.tail.get_mut() = capacity;
        *queue.threshold.get_mut() = IndexQueue::threshold(capacity, slots);

        queue
    }

    pub fn push(&self, index: usize, order: usize) {
        let capacity = 1 << order;
        let slots = capacity * 2;

        'next: loop {
            // acquire a slot
            let tail = self.tail.fetch_add(1, Ordering::Relaxed);
            let tail_index = cache_remap!(tail, order, slots);
            let cycle = (tail << 1) | (2 * slots - 1);

            let mut slot = unsafe { self.slots.get_unchecked(tail_index).load(Ordering::Acquire) };

            'retry: loop {
                let slot_cycle = slot | (2 * slots - 1);

                // the slot is from a newer cycle, move to
                // the next one
                if wrapping_cmp!(slot_cycle, >=, cycle) {
                    continue 'next;
                }

                // we can safely read the entry if either:
                // - the entry is unused and safe
                // - the entry is unused and _unsafe_ but
                //   the head is behind the tail, meaning
                //   all active readers are still behind
                if slot == slot_cycle
                    || (slot == slot_cycle ^ slots
                        && wrapping_cmp!(self.head.load(Ordering::Acquire), <=, tail))
                {
                    // set the safety bit and cycle
                    let new_slot = index ^ (slots - 1);
                    let new_slot = cycle ^ new_slot;

                    unsafe {
                        if let Err(new) =
                            self.slots.get_unchecked(tail_index).compare_exchange_weak(
                                slot,
                                new_slot,
                                Ordering::AcqRel,
                                Ordering::Acquire,
                            )
                        {
                            slot = new;
                            continue 'retry;
                        }
                    }

                    let threshold = IndexQueue::threshold(capacity, slots);

                    // reset the threshold
                    if self.threshold.load(Ordering::Acquire) != threshold {
                        self.threshold.store(threshold, Ordering::Release);
                    }

                    return;
                }

                continue 'next;
            }
        }
    }

    pub fn pop(&self, order: usize) -> Option<usize> {
        // the queue is empty
        if self.threshold.load(Ordering::Acquire) < 0 {
            return None;
        }

        let slots = 1 << (order + 1);

        'next: loop {
            // acquire a slot
            let head = self.head.fetch_add(1, Ordering::Relaxed);
            let head_index = cache_remap!(head, order, slots);
            let cycle = (head << 1) | (2 * slots - 1);

            let mut spun = 0;

            'spin: loop {
                let mut slot =
                    unsafe { self.slots.get_unchecked(head_index).load(Ordering::Acquire) };

                'retry: loop {
                    let slot_cycle = slot | (2 * slots - 1);

                    // if the cycles match, we can read this slot
                    if slot_cycle == cycle {
                        unsafe {
                            // mark as unused, but preserve the safety bit
                            self.slots
                                .get_unchecked(head_index)
                                .fetch_or(slots - 1, Ordering::AcqRel);
                        }

                        // extract the index (ignore cycle and safety bit)
                        return Some(slot & (slots - 1));
                    }

                    if wrapping_cmp!(slot_cycle, <, cycle) {
                        // otherwise, we have to update the cycle
                        let new_slot = if (slot | slots) == slot_cycle {
                            // spin for a bit before invalidating
                            // the slot for writers from a previous
                            // cycle in case they arrive soon
                            if spun < SPIN_LIMIT {
                                spun += 1;

                                for _ in (0..(spun.pow(2))) {
                                    hint::spin_loop()
                                }

                                continue 'spin;
                            }

                            // the slot is unused, preserve the safety bit
                            cycle ^ (!slot & slots)
                        } else {
                            // mark the slot as unsafe
                            let new_entry = slot & !slots;

                            if slot == new_entry {
                                break 'retry;
                            }

                            new_entry
                        };

                        unsafe {
                            if let Err(new) =
                                self.slots.get_unchecked(head_index).compare_exchange_weak(
                                    slot,
                                    new_slot,
                                    Ordering::AcqRel,
                                    Ordering::Acquire,
                                )
                            {
                                slot = new;
                                continue 'retry;
                            }
                        }
                    }

                    break 'retry;
                }

                // check if the queue is empty
                let tail = self.tail.load(Ordering::Acquire);

                // if the head overtook the tail, push the tail forward
                if tail <= head + 1 {
                    self.catchup(tail, head + 1);
                    self.threshold.fetch_sub(1, Ordering::AcqRel);
                    return None;
                }

                if self.threshold.fetch_sub(1, Ordering::AcqRel) <= 0 {
                    return None;
                }

                continue 'next;
            }
        }
    }

    fn catchup(&self, mut tail: usize, mut head: usize) {
        while let Err(found) =
            self.tail
                .compare_exchange_weak(tail, head, Ordering::AcqRel, Ordering::Acquire)
        {
            tail = found;
            head = self.head.load(Ordering::Acquire);

            if wrapping_cmp!(tail, >=, head) {
                break;
            }
        }
    }

    pub fn len(&self, order: usize) -> usize {
        let capacity = 1 << order;

        loop {
            let tail = self.tail.load(Ordering::Acquire);
            let head = self.head.load(Ordering::Acquire);

            // make sure we have consistent values to work with
            if self.tail.load(Ordering::Acquire) == tail {
                let head_index = head & (capacity - 1);
                let tail_index = tail & (capacity - 1);

                break if head_index < tail_index {
                    tail_index - head_index
                } else if head_index > tail_index {
                    capacity - (head_index - tail_index)
                } else if tail == head {
                    0
                } else {
                    capacity
                };
            }
        }
    }

    #[inline(always)]
    fn threshold(half: usize, cap: usize) -> isize {
        ((half + cap) - 1) as isize
    }
}

macro_rules! wrapping_cmp {
    ($x:expr, $op:tt, $y:expr) => {
        ((($x).wrapping_sub($y)) as isize) $op 0
    }
}

macro_rules! cache_remap {
    ($i:expr, $order:expr, $n:expr) => {
        raw_cache_remap!($i, ($order) + 1, $n)
    };
}

macro_rules! raw_cache_remap {
    ($i:expr, $order:expr, $n:expr) => {
        ((($i) & (($n) - 1)) >> (($order) - MIN)) | ((($i) << MIN) & (($n) - 1))
    };
}

pub(self) use {cache_remap, raw_cache_remap, wrapping_cmp};

const CACHE_SHIFT: usize = 7; // TODO: non-x86

#[cfg(target_pointer_width = "32")]
const MIN: usize = CACHE_SHIFT - 2;

#[cfg(target_pointer_width = "64")]
const MIN: usize = CACHE_SHIFT - 3;

#[cfg(target_pointer_width = "128")]
const MIN: usize = CACHE_SHIFT - 4;
