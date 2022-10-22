use crate::raw::parking;
use crate::raw::util::{assert_valid_capacity, CachePadded};

use std::cell::UnsafeCell;
use std::mem::MaybeUninit;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::{hint, ptr};

// A bounded queue built on two index queues.
pub struct Queue<T> {
    free: IndexQueue,
    elements: IndexQueue,
    slots: Box<[UnsafeCell<MaybeUninit<T>>]>,
}

unsafe impl<T: Send> Send for Queue<T> {}
unsafe impl<T: Send> Sync for Queue<T> {}

impl<T> Queue<T> {
    pub fn new(capacity: usize) -> Self {
        assert_valid_capacity(capacity);
        if capacity >= MAX_CAPACITY {
            panic!("exceeded maximum queue capacity of {}", MAX_CAPACITY);
        }

        Queue {
            slots: (0..capacity)
                .map(|_| UnsafeCell::new(MaybeUninit::uninit()))
                .collect(),
            free: IndexQueue::full(capacity),
            elements: IndexQueue::empty(capacity),
        }
    }

    pub fn push(&self, value: T) -> Result<(), T> {
        match self.free.pop() {
            Some(elem) => unsafe {
                self.slots
                    .get_unchecked(elem)
                    .get()
                    .write(MaybeUninit::new(value));
                self.elements.push(elem);
                Ok(())
            },
            None => Err(value),
        }
    }

    pub fn pop(&self) -> Option<T> {
        let elem = self.elements.pop()?;
        unsafe {
            let value = self.slots.get_unchecked(elem).get().read().assume_init();
            self.free.push(elem);
            Some(value)
        }
    }

    pub fn can_push(&self) -> bool {
        self.free.can_pop()
    }

    pub fn can_pop(&self) -> bool {
        self.elements.can_pop()
    }
}

impl<T> Drop for Queue<T> {
    fn drop(&mut self) {
        while let Some(elem) = self.elements.pop() {
            unsafe { ptr::drop_in_place(self.slots.get_unchecked(elem).get().cast::<T>()) }
        }
    }
}

pub const MAX_CAPACITY: usize = u32::MAX as _;

const SPIN_LIMIT: usize = 6;

pub struct IndexQueue {
    head: CachePadded<AtomicUsize>,
    tail: CachePadded<AtomicUsize>,
    // Slots stores an index, a cycle, and a special 'safety' marker:
    //
    //        -------------------------------
    // BITS:  | 64... | order+1 | order...0 |
    //        -------------------------------
    // VALUE: | cycle |  safe?  |   index   |
    //        -------------------------------
    //
    // Where `order` is the power of 2 capacity of the queue.
    // If all the index bits are set to 1, the slot is vacant.
    //
    // The number of slots is double the capacity of the queue.
    slots: CachePadded<Box<[AtomicUsize]>>,
}

impl IndexQueue {
    pub fn empty(capacity: usize) -> IndexQueue {
        // the initial slot state, vacant and safe
        const INIT: usize = !0;

        IndexQueue {
            head: CachePadded(AtomicUsize::new(0)),
            tail: CachePadded(AtomicUsize::new(0)),
            slots: CachePadded((0..capacity * 2).map(|_| AtomicUsize::new(INIT)).collect()),
        }
    }

    pub fn full(capacity: usize) -> IndexQueue {
        let mut queue = IndexQueue::empty(capacity);

        for i in 0..capacity {
            queue.slots[i] = AtomicUsize::new(i);
        }

        *queue.tail.get_mut() = capacity;

        queue
    }

    pub fn push(&self, index: usize) {
        let slots = self.slots.len();

        // note that the fact that we're pushing means we popped a corresponding
        // index, so we know for sure the queue is not full
        loop {
            // acquire a slot
            let tail = self.tail.fetch_add(1, Ordering::Relaxed);
            let tail_index = tail & (slots - 1);
            let tail_cycle = (tail << 1) | cycle_mask!(slots);

            let mut slot = unsafe { self.slots.get_unchecked(tail_index).load(Ordering::Acquire) };

            'retry: loop {
                let slot_cycle = slot | cycle_mask!(slots);

                // our reader has already visited this slot
                // and updated its cycle, we have to skip it
                if wrapping_cmp!(slot_cycle, >=, tail_cycle) {
                    break 'retry;
                }

                // we can safely write to the slot if either:
                // - the slot is vacant and safe
                // - the slot is vacant and marked as *unsafe*, but our reader
                // has not started yet, so we can restore the slot's safety
                if slot == slot_cycle
                    || (slot == slot_cycle ^ slots
                        && wrapping_cmp!(self.head.load(Ordering::Acquire), <=, tail))
                {
                    // mark as safe and set the cycle
                    let index = tail_cycle ^ index ^ (slots - 1);

                    if let Err(found) = unsafe { self.slots.get_unchecked(tail_index) }
                        .compare_exchange_weak(slot, index, parking::RELEASE, Ordering::Acquire)
                    {
                        slot = found;
                        continue 'retry;
                    }

                    return;
                }

                break 'retry;
            }
        }
    }

    pub fn can_pop(&self) -> bool {
        let slots = self.slots.len();

        // load the current head
        let head = self.head.load(Ordering::Relaxed);
        let head_index = head & (slots - 1);
        let head_cycle = (head << 1) | cycle_mask!(slots);

        // load the current slot
        let slot = unsafe { self.slots.get_unchecked(head_index).load(Ordering::Relaxed) };
        let slot_cycle = slot | cycle_mask!(slots);

        // if cycles match, we can read from the slot
        slot_cycle == head_cycle
    }

    pub fn pop(&self) -> Option<usize> {
        let slots = self.slots.len();

        loop {
            // acquire a slot
            let head = self.head.fetch_add(1, Ordering::Relaxed);
            let head_index = head & (slots - 1);
            let head_cycle = (head << 1) | cycle_mask!(slots);

            let mut spun = 0;
            'spin: loop {
                let mut slot =
                    unsafe { self.slots.get_unchecked(head_index).load(Ordering::Acquire) };

                'retry: loop {
                    let slot_cycle = slot | cycle_mask!(slots);

                    // the cycles match, we can safely read from this slot
                    if slot_cycle == head_cycle {
                        // mark as unused, preserving the cycle and safety
                        unsafe {
                            self.slots
                                .get_unchecked(head_index)
                                .fetch_or(slots - 1, parking::RELEASE);
                        }
                        return Some(slot & (slots - 1));
                    }

                    // this slot is from an earlier cycle
                    if wrapping_cmp!(slot_cycle, <, head_cycle) {
                        // if the slot is unused, we can update the cycle
                        // for the *next* cycle's writer to use
                        let new_slot = if (slot | slots) == slot_cycle {
                            // but first, spin for a bit before invalidating
                            // the slot in case the writer arrives soon
                            if spun <= SPIN_LIMIT {
                                for _ in 0..spun.pow(2) {
                                    hint::spin_loop()
                                }

                                spun += 1;
                                continue 'spin;
                            }

                            // make sure to preserve the safety bit
                            head_cycle ^ (!slot & slots)
                        }
                        // if the slot is occupied by a previous cycle, we need to
                        // mark it as unsafe so the future writer knows to skip it
                        else {
                            let unsafe_slot = slot & !slots;

                            // the slot is already unsafe from a previous cycle
                            if slot == unsafe_slot {
                                break 'retry;
                            }

                            unsafe_slot
                        };

                        if let Err(found) = unsafe { self.slots.get_unchecked(head_index) }
                            .compare_exchange_weak(
                                slot,
                                new_slot,
                                Ordering::AcqRel,
                                Ordering::Acquire,
                            )
                        {
                            slot = found;
                            continue 'retry;
                        }
                    }

                    break 'retry;
                }

                let tail = self.tail.load(Ordering::Acquire);

                // the queue is empty, help the tail forward
                if tail <= head + 1 {
                    self.help_tail(tail, head + 1);
                    return None;
                }

                break 'spin;
            }
        }
    }

    fn help_tail(&self, mut tail: usize, mut head: usize) {
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
}

// Masks a slot's index and safety bit.
//
// Note that the head/tail must be shifted to account for the safety bit.
macro_rules! cycle_mask {
    ($slots:ident) => {
        (2 * $slots - 1)
    };
}

macro_rules! wrapping_cmp {
    ($x:expr, $op:tt, $y:expr) => {
        ((($x).wrapping_sub($y)) as isize) $op 0
    }
}

// TODO(ibraheem): re-evaluate cache shifting, results seems inconsistent
// macro_rules! cache_remap {
//     ($i:expr, $order:expr, $n:expr) => {
//         raw_cache_remap!($i, ($order) + 1, $n)
//     };
// }
//
// macro_rules! raw_cache_remap {
//     ($i:expr, $order:expr, $n:expr) => {
//         ((($i) & (($n) - 1)) >> (($order) - MIN)) | ((($i) << MIN) & (($n) - 1))
//     };
// }

use {cycle_mask, wrapping_cmp};
