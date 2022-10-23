use crate::raw::parking;
use crate::raw::util::{self, *};

use std::cell::{Cell, UnsafeCell};
use std::mem::{ManuallyDrop, MaybeUninit};
use std::sync::atomic::{AtomicPtr, AtomicU32, AtomicU8, Ordering};
use std::{hint, ptr};

pub const MAX_SENDERS: usize = (1 << TAG_BITS) - BLOCK_SIZE + 1;

pub struct Queue<T> {
    // pointer and index into the head of the queue
    head: CachePadded<Cell<(*mut Block<T>, usize)>>,
    // pointer to the tail of the queue, tagged with the index
    tail: CachePadded<AtomicPtr<Block<T>>>,
    // a cached copy of the tail pointer
    cached_tail: AtomicPtr<Block<T>>,
}

unsafe impl<T: Send> Send for Queue<T> {}

const SPIN_READ: usize = 16;

impl<T> Queue<T> {
    pub fn new() -> Queue<T> {
        let block = Block::new();

        Queue {
            head: CachePadded(Cell::new((block, 0))),
            tail: CachePadded(AtomicPtr::new(block)),
            cached_tail: AtomicPtr::new(block),
        }
    }

    pub unsafe fn is_empty(&self) -> bool {
        let cached_tail = self.cached_tail.load(Ordering::Acquire);
        let (head, head_index) = self.head.get();

        // if the head and tail are not at the same block, we can be
        // sure the queue is empty without checking indices
        if head != cached_tail {
            return false;
        }

        // otherwise we have to load the tail
        let (tail, tail_index) = self.tail.load(Ordering::Relaxed).split(BLOCK_MASK);

        // update the cached tail if it is behind
        if tail != cached_tail {
            self.cached_tail
                .compare_exchange(cached_tail, tail, Ordering::Release, Ordering::Relaxed)
                .ok();

            // the tail is ahead, so the queue cannot be empty
            return false;
        }

        // is the tail behind the head?
        tail_index <= head_index
    }

    pub unsafe fn pop(&self) -> Option<T> {
        loop {
            if self.is_empty() {
                return None;
            }

            let (head, index) = self.head.get();

            // still in this block, read the slot
            if index < BLOCK_SIZE {
                let slot = head.deref().slots.get_unchecked(index);

                for _ in 0..SPIN_READ {
                    // check if the slot has been written to
                    if slot.state.load(Ordering::Acquire) & WRITTEN == WRITTEN {
                        // if it has, read the value and update the state
                        let value = slot.value.get().read().assume_init();
                        slot.state.fetch_add(CONSUMED, Ordering::Release);
                        self.head.set((head, index + 1));
                        return Some(value);
                    }

                    hint::spin_loop();
                }

                // the slot is empty
                return None;
            }

            // if the tail has not advanced there is no next block
            let tail = self.tail.load(Ordering::Acquire).mask(BLOCK_MASK);
            if head == tail {
                return None;
            }

            // if it has, the next block must have been set. advance the head
            let next = head.deref().next.load(Ordering::Acquire);
            self.head.set((next, 0));

            // we will never touch the old block again, mark the head as advanced
            let state = head.deref().state.fetch_add(HEAD_ADVANCED, Ordering::AcqRel);

            // if the tail also advanced we can safely free the block
            if state == TAIL_ADVANCED {
                let _ = Box::from_raw(head);
            }
        }
    }

    pub fn push(&self, value: T) {
        let value = ManuallyDrop::new(value);

        loop {
            // acquire a unique index
            let (tail, index) = self.tail.fetch_add(1, Ordering::Acquire).split(BLOCK_MASK);

            // still in this block, simply write to the slot and update the state
            if index < BLOCK_SIZE {
                unsafe {
                    let slot = tail.deref().slots.get_unchecked(index);
                    slot.value.get().cast::<T>().copy_from_nonoverlapping(&*value, 1);
                    slot.state.fetch_add(WRITTEN, parking::RELEASE);
                    return;
                }
            }

            // check if a new tail has been already set
            let current_tail = self.tail.load(Ordering::Relaxed);
            if tail != current_tail.mask(BLOCK_MASK) {
                // if it has, help reclaim the old one and try again in the new block
                unsafe { Block::try_reclaim_tail(tail, None) };
                continue;
            }

            // otherwise, try to load the next block ourselves
            let next = unsafe { tail.deref().next.load(Ordering::Relaxed) };

            // if there is no next block, we have to allocate one
            if next.is_null() {
                unsafe {
                    let new_block = Block::new();

                    // write the value to the first slot
                    new_block.deref_mut().slots[0] = Slot {
                        value: UnsafeCell::new(MaybeUninit::new(ptr::read(&*value))),
                        state: AtomicU8::new(WRITTEN),
                    };

                    // try to write our block to `next`
                    let (won, next) = match tail.deref().next.compare_exchange(
                        ptr::null_mut(),
                        new_block,
                        parking::RELEASE,
                        Ordering::Relaxed,
                    ) {
                        Ok(_) => (true, new_block),
                        // someone else beat us, use their block instead
                        Err(found) => (false, found),
                    };

                    // help set the tail to the new block, whether ours, or the one we found
                    let next_tagged = next.map_addr(|addr| addr | 1);
                    let final_count = Block::compare_exchange(&self.tail, current_tail, next_tagged).ok();

                    // help update the cache
                    self.cached_tail
                        .compare_exchange(tail, next, Ordering::Release, Ordering::Relaxed)
                        .ok();

                    // help reclaim the old tail
                    Block::try_reclaim_tail(tail, final_count);

                    // if we lost the race then our block was never used.. deallocate it and retry
                    if !won {
                        let _ = Box::from_raw(new_block);
                        continue;
                    }

                    // otherwise we wrote the value to the first slot, we're done
                    return;
                }
            }

            // if the next block was already allocated, try to update the tail. the first slot
            // must have been written to by the writer who created the block, so skip it
            let next_tagged = next.map_addr(|addr| addr | 1);
            let final_count = Block::compare_exchange(&self.tail, current_tail, next_tagged).ok();

            // help update the cached tail
            self.cached_tail
                .compare_exchange(tail, next, Ordering::Release, Ordering::Relaxed)
                .ok();

            // help reclaim the old tail
            unsafe { Block::try_reclaim_tail(tail, final_count) };
        }
    }
}

// the number of tag bits used for the index
const TAG_BITS: usize = 12;
// mask for tag bits
const BLOCK_MASK: usize = !((1 << TAG_BITS) - 1);
// number of slots in a block
const BLOCK_SIZE: usize = 1024;
// the final count is stored in the higher 16 bits
const FINAL_COUNT_SHIFT: u32 = 16;
// the current count is stored in the lower bits
const CURRENT_COUNT_MASK: u32 = 0xFFFF;

// the tail of the queue no longer points to this block
const TAIL_ADVANCED: u8 = 0b010;
// the head of the queue no longer points to this block
const HEAD_ADVANCED: u8 = 0b100;

#[repr(align(4096))] // (1 << Block::TAG_BITS)
struct Block<T> {
    slots: [Slot<T>; BLOCK_SIZE],
    // a counter used to reclaim the block
    //
    // after advancing the tail, the index tag is set as the 'final count', representing
    // all possible writers that still have access to the tail. each of the writers increments
    // the 'current count'. the last writer to increment the count and observe the final count
    // can mark the tail as advanced
    count: AtomicU32,
    // stores whether the head/tail have advanced and no longer point to this block,
    // which means the block can be freed
    state: AtomicU8,
    // a pointer to the next block in the queue
    next: AtomicPtr<Block<T>>,
}

// an element was written to the slot
const WRITTEN: u8 = 0b001;
// the slot was consumed by a reader
const CONSUMED: u8 = 0b010;

struct Slot<T> {
    value: UnsafeCell<MaybeUninit<T>>,
    state: AtomicU8,
}

impl<T> Block<T> {
    // allocate a new block
    fn new() -> *mut Block<T> {
        let block: *mut Block<T> = unsafe { Box::into_raw(util::box_zeroed()) };
        // make sure we have enough tag bits
        assert_eq!(block.unmask(BLOCK_MASK), 0);
        block
    }

    // try to reclaim the tail block of the queue
    unsafe fn try_reclaim_tail(block: *mut Block<T>, final_count: Option<u32>) {
        let count = match final_count {
            // if the final count was observed, set it, and increment the current count
            Some(final_count) => (final_count << FINAL_COUNT_SHIFT) + 1,
            // otherwise the final count has already been set, just increment the current count
            None => 1,
        };

        // update the count and retreive the current and final counts
        let count = block.deref().count.fetch_add(count, Ordering::Relaxed);
        let final_count = final_count.unwrap_or(count >> FINAL_COUNT_SHIFT);
        let current_count = (count & CURRENT_COUNT_MASK) + 1;

        // if they are equal, all writers have incremented the count and we mark the tail as
        // advanced
        if current_count == final_count {
            let state = block.deref().state.fetch_add(TAIL_ADVANCED, Ordering::AcqRel);

            // if the head has also advanced, we can safely free the block
            if state == HEAD_ADVANCED {
                let _ = Box::from_raw(block);
            }
        }
    }

    // sets the block pointer to `new` if it is still equal to `old`, ignoring any changes
    // to the tag
    //
    // on success this returns the final index count of the old block
    fn compare_exchange(
        ptr: &AtomicPtr<Block<T>>,
        old: *mut Block<T>,
        new: *mut Block<T>,
    ) -> Result<u32, *mut Block<T>> {
        let mut current = old;

        while let Err(found) = ptr.compare_exchange(current, new, Ordering::Release, Ordering::Relaxed) {
            // only fail if the pointer itself changed
            if found.mask(BLOCK_MASK) != current.mask(BLOCK_MASK) {
                return Err(found);
            }

            // if just the tag changed, the new pointer still hasn't been set, try again
            current = found;
        }

        // retreive the final count, ignoring the ones that actually wrote to the block,
        // because we can't reclaim the block until the reader finishes anyways
        Ok((current.unmask(BLOCK_MASK) - BLOCK_SIZE) as u32)
    }
}

impl<T> Drop for Queue<T> {
    fn drop(&mut self) {
        unsafe {
            while let Some(value) = self.pop() {
                drop(value);
            }

            let (head, _) = self.head.get();
            assert_eq!(head, self.tail.load(Ordering::Relaxed).mask(BLOCK_MASK));

            if !head.is_null() {
                let _ = Box::from_raw(head);
            }
        }
    }
}
