use crate::raw::util::{assert_valid_capacity, CachePadded};

use std::cell::UnsafeCell;
use std::hint;
use std::mem::MaybeUninit;
use std::sync::atomic::{self, AtomicUsize, Ordering};

/// A bounded single-producer multi-consumer queue.
pub struct Queue<T> {
    // The head/tail of the queue, with the current cycle packed into the upper bits.
    head: CachePadded<AtomicUsize>,
    tail: CachePadded<AtomicUsize>,

    // The slots.
    slots: Box<[Slot<T>]>,

    // A single cycle around the queue.
    one_cycle: usize,
}

/// A slot in a queue.
struct Slot<T> {
    // The current stamp.
    stamp: AtomicUsize,

    // The value in this slot.
    value: UnsafeCell<MaybeUninit<T>>,
}

const MAX_BACKOFF: usize = 6;

unsafe impl<T: Send> Sync for Queue<T> {}
unsafe impl<T: Send> Send for Queue<T> {}

impl<T> Queue<T> {
    pub fn new(capacity: usize) -> Queue<T> {
        assert_valid_capacity(capacity);

        let slots: Box<[Slot<T>]> = (0..capacity)
            .map(|i| {
                Slot {
                    // set the stamp equal to the tail index that will write to it
                    stamp: AtomicUsize::new(i),
                    value: UnsafeCell::new(MaybeUninit::uninit()),
                }
            })
            .collect();

        Queue {
            slots,
            // a cycle is the smallest power of two greater than `cap`
            one_cycle: (capacity + 1).next_power_of_two(),
            head: CachePadded(AtomicUsize::new(0)),
            tail: CachePadded(AtomicUsize::new(0)),
        }
    }

    pub fn can_push(&self) -> bool {
        // load the current tail
        let tail = self.tail.load(Ordering::Relaxed);
        let tail_index = tail & (self.one_cycle - 1);

        // load the associated slot
        let slot = unsafe { self.slots.get_unchecked(tail_index) };
        let stamp = slot.stamp.load(Ordering::Acquire);

        // if the tail and slot match, we can push
        tail == stamp
    }

    pub unsafe fn push(&self, value: T) -> Result<(), T> {
        let mut backoff = 0_usize;
        let mut tail = self.tail.load(Ordering::Relaxed);

        loop {
            let tail_index = tail & (self.one_cycle - 1);
            let tail_cycle = tail & !(self.one_cycle - 1);

            let next_tail = if tail_index + 1 < self.slots.len() {
                // same cycle, increment the index
                tail + 1
            } else {
                // move to the next cycle
                tail_cycle.wrapping_add(self.one_cycle)
            };

            let slot = self.slots.get_unchecked(tail_index);
            let stamp = slot.stamp.load(Ordering::Acquire);

            // if the tail and the stamp match, we can push
            if tail == stamp {
                // move the tail
                self.tail.store(next_tail, Ordering::SeqCst);

                // write the value into the slot
                slot.value.get().write(MaybeUninit::new(value));
                slot.stamp.store(tail + 1, Ordering::Release);

                return Ok(());
            }
            // if the stamp is from the previous cycle, the queue is probably full
            else if stamp.wrapping_add(self.one_cycle) == tail + 1 {
                atomic::fence(Ordering::SeqCst);
                let head = self.head.load(Ordering::Relaxed);

                // the head is also a cycle behind the tail, the queue is full
                if head.wrapping_add(self.one_cycle) == tail {
                    return Err(value);
                }

                // otherwise, the reader moved the head forward but hasn't
                // read the value yet. spin, waiting for them to update the stamp
                backoff = (backoff + 1).max(MAX_BACKOFF);
                for _ in 0..backoff.pow(2) {
                    hint::spin_loop();
                }

                tail = self.tail.load(Ordering::Relaxed);
            } else {
                // another writer beat us to the slot and updated the stamp,
                // load the new tail and retry
                tail = self.tail.load(Ordering::Relaxed);
            }
        }
    }

    pub fn pop(&self) -> Option<T> {
        let mut backoff = 0_usize;
        let mut head = self.head.load(Ordering::Relaxed);

        loop {
            let head_index = head & (self.one_cycle - 1);
            let head_cycle = head & !(self.one_cycle - 1);

            let slot = unsafe { self.slots.get_unchecked(head_index) };
            let stamp = slot.stamp.load(Ordering::Acquire);

            // if the the stamp is ahead of the head by 1, we can pop
            if head + 1 == stamp {
                let new = if head_index + 1 < self.slots.len() {
                    // same cycle, increment the index
                    head + 1
                } else {
                    // move to the next cycle
                    head_cycle.wrapping_add(self.one_cycle)
                };

                // try moving the head
                match self.head.compare_exchange_weak(
                    head,
                    new,
                    Ordering::SeqCst,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => {
                        // we claimed this slot, read the value
                        let value = unsafe { slot.value.get().read().assume_init() };

                        // and update the cycle for the next reader
                        slot.stamp
                            .store(head.wrapping_add(self.one_cycle), Ordering::Release);

                        return Some(value);
                    }
                    Err(found) => {
                        // someone else beat us, try again
                        head = found;
                        continue;
                    }
                }
            }
            // the stamp is from the previous reader, the channel is probably empty
            else if stamp == head {
                atomic::fence(Ordering::SeqCst);
                let tail = self.tail.load(Ordering::Relaxed);

                // the tail is equal to the head, the channel is empty
                if tail == head {
                    return None;
                }

                // otherwise, the writer moved the tail forward but hasn't
                // written the value yet. spin, waiting for them to update the stamp
                backoff = (backoff + 1).max(MAX_BACKOFF);
                for _ in 0..backoff.pow(2) {
                    hint::spin_loop();
                }

                head = self.head.load(Ordering::Relaxed);
            } else {
                // another reader beat us to the slot and updated the stamp,
                // load the new head and retry
                head = self.head.load(Ordering::Relaxed);
            }
        }
    }
}

impl<T> Drop for Queue<T> {
    fn drop(&mut self) {
        while let Some(_) = self.pop() {}
    }
}
