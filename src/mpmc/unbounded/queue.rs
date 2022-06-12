use crate::util::{self, CachePadded, FetchAddPtr, StrictProvenance, UnsafeDeref};

use std::cell::UnsafeCell;
use std::mem::{ManuallyDrop, MaybeUninit};
use std::ptr;
use std::sync::atomic::{AtomicPtr, AtomicU32, AtomicU8, Ordering};

pub const MAX_SENDERS: usize = (1 << Block::TAG_BITS) - Block::SLOTS + 1;
pub const MAX_RECEIVERS: usize = MAX_SENDERS / 2;

const SPIN_READ: usize = 16;

pub struct Queue<T> {
    head: CachePadded<AtomicPtr<Block<T>>>,
    tail: CachePadded<AtomicPtr<Block<T>>>,
    cached_tail: AtomicPtr<Block<T>>,
}

unsafe impl<T> Send for Queue<T> {}
unsafe impl<T> Sync for Queue<T> {}

impl<T> Queue<T> {
    pub fn new() -> Queue<T> {
        let block = Block::alloc();

        Queue {
            head: CachePadded(AtomicPtr::new(block)),
            tail: CachePadded(AtomicPtr::new(block)),
            cached_tail: AtomicPtr::new(block),
        }
    }

    pub fn pop(&self) -> Option<T> {
        loop {
            if self.is_empty() {
                return None;
            }

            let current = self.head.fetch_add(1, Ordering::AcqRel);
            let index = current.addr() & Block::INDEX_MASK;
            let head = current.map_addr(|addr| addr & !Block::INDEX_MASK);

            if index < Block::SLOTS {
                unsafe {
                    let slot = head.deref().slots.get_unchecked(index);

                    for _ in 0..SPIN_READ {
                        if slot.state.load(Ordering::Acquire) & WRITTEN == WRITTEN {
                            let value = slot.value.get().read().assume_init();
                            match slot.state.fetch_add(CONSUMED, Ordering::Release) {
                                WRITTEN => return Some(value),
                                READER_RESUME => {
                                    Block::try_reclaim(head, index + 1);
                                    return Some(value);
                                }
                                _ => unreachable!(),
                            }
                        }

                        std::hint::spin_loop();
                    }

                    match slot.state.fetch_add(INVALID, Ordering::Acquire) {
                        WRITTEN => {
                            let value = slot.value.get().read().assume_init();

                            if slot.state.fetch_add(CONSUMED, Ordering::Release)
                                == INVALID | READER_RESUME
                            {
                                Block::try_reclaim(head, index + 1);
                            }

                            return Some(value);
                        }
                        READER_RESUME => {
                            let value = slot.value.get().read().assume_init();
                            slot.state.fetch_add(CONSUMED, Ordering::Release);
                            Block::try_reclaim(head, index + 1);
                            return Some(value);
                        }
                        _ => {
                            if slot.state.fetch_add(CONSUMED, Ordering::Release)
                                == INVALID | READER_RESUME
                            {
                                Block::try_reclaim(head, index + 1);
                            }

                            continue;
                        }
                    };
                }
            }

            if index == Block::SLOTS {
                unsafe { Block::try_reclaim(head, 0) };
            }

            let tail = self
                .tail
                .load(Ordering::Acquire)
                .map_addr(|addr| addr & !Block::INDEX_MASK);

            if head == tail {
                unsafe { Block::try_reclaim_head_slow(head, None) };
                return None;
            }

            unsafe {
                let next = head.deref().next.load(Ordering::Acquire);

                let current = (current as usize).wrapping_add(1) as *mut _;
                let final_count = Block::compare_exchange(&self.head, current, next, head);

                Block::try_reclaim_head_slow(head, final_count);
                continue;
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        let cached_tail = self.cached_tail.load(Ordering::Acquire);

        let head = self.head.fetch_add(0, Ordering::Relaxed);
        let head_index = head.addr() & Block::INDEX_MASK;
        let head = head.map_addr(|addr| addr & !Block::INDEX_MASK);

        if head != cached_tail {
            return false;
        }

        let tail = self.tail.load(Ordering::Relaxed);
        let tail_index = tail.addr() & Block::INDEX_MASK;
        let tail = tail.map_addr(|addr| addr & !Block::INDEX_MASK);

        if tail != cached_tail {
            let _ = self.cached_tail.compare_exchange(
                cached_tail,
                tail,
                Ordering::Release,
                Ordering::Relaxed,
            );

            return false;
        }

        tail_index <= head_index
    }

    pub fn push(&self, value: T) {
        let value = ManuallyDrop::new(value);

        loop {
            let tail = self.tail.fetch_add(1, Ordering::Acquire);
            let index = tail.addr() & Block::INDEX_MASK;
            let tail = tail.map_addr(|addr| addr & !Block::INDEX_MASK);

            if index < Block::SLOTS {
                unsafe {
                    let slot = tail.deref().slots.get_unchecked(index);

                    slot.value
                        .get()
                        .deref_mut()
                        .as_mut_ptr()
                        .copy_from_nonoverlapping(&*value, 1);

                    match slot.state.fetch_add(WRITTEN, Ordering::Release) {
                        UNINIT | RESUME => return,
                        WRITER_RESUME => Block::try_reclaim(tail, index + 1),
                        _ => {}
                    }

                    continue;
                }
            }

            let current = self.tail.load(Ordering::Relaxed);

            if tail != current.map_addr(|addr| addr & !Block::INDEX_MASK) {
                unsafe { Block::try_reclaim_tail_slow(tail, None) };
                continue;
            }

            let next = unsafe { tail.deref().next.load(Ordering::Relaxed) };

            if next.is_null() {
                unsafe {
                    let block_alloc = Block::alloc();
                    block_alloc.deref_mut().slots[0] = Slot {
                        value: UnsafeCell::new(MaybeUninit::new(ptr::read(&*value))),
                        state: AtomicU8::new(WRITTEN),
                    };

                    let (res, next) = match tail.deref().next.compare_exchange(
                        ptr::null_mut(),
                        block_alloc,
                        Ordering::Release,
                        Ordering::Relaxed,
                    ) {
                        Ok(_) => (Ok(()), block_alloc),
                        Err(found) => (Err(()), found),
                    };

                    let next_tagged = (next as usize) | Block::INDEX_MASK & 1;
                    let final_count =
                        Block::compare_exchange(&self.tail, current, next_tagged as _, tail);

                    let _ = self.cached_tail.compare_exchange(
                        tail,
                        next,
                        Ordering::Release,
                        Ordering::Relaxed,
                    );

                    Block::try_reclaim_tail_slow(tail, final_count);

                    match res {
                        Ok(_) => return,
                        Err(_) => {
                            Block::dealloc(block_alloc);
                            continue;
                        }
                    }
                }
            } else {
                let next_tagged = (next as usize) | Block::INDEX_MASK & 1;
                let final_count =
                    Block::compare_exchange(&self.tail, current, next_tagged as _, tail);

                let _ = self.cached_tail.compare_exchange(
                    tail,
                    next,
                    Ordering::Release,
                    Ordering::Relaxed,
                );

                unsafe { Block::try_reclaim_tail_slow(tail, final_count) };
                continue;
            }
        }
    }
}

impl<T> Drop for Queue<T> {
    fn drop(&mut self) {
        unsafe {
            while let Some(value) = self.pop() {
                drop(value);
            }

            let head = self
                .head
                .get_mut()
                .map_addr(|addr| addr & !Block::INDEX_MASK);

            assert_eq!(
                head,
                self.tail
                    .load(Ordering::Relaxed)
                    .map_addr(|addr| addr & !Block::INDEX_MASK)
            );

            if !head.is_null() {
                Block::dealloc(head);
            }
        }
    }
}

#[repr(align(4096))] // (1 << Block::TAG_BITS)
struct Block<T> {
    slots: [Slot<T>; Block::SLOTS],
    push_count: AtomicU32,
    pop_count: AtomicU32,
    reclamation: AtomicU8,
    next: AtomicPtr<Block<T>>,
}

impl Block<()> {
    const TAG_BITS: usize = 12;
    const SLOTS: usize = 1024;
    const INDEX_MASK: usize = (1 << Block::TAG_BITS) - 1;
    const FINAL_COUNT_SHIFT: u32 = 16;
    const CURRENT_COUNT_MASK: u32 = 0xFFFF;

    // all slots have been consumed
    const CONSUMED: u8 = 0b001;
    // the tail of the queue no longer points to this block
    const TAIL_ADVANCED: u8 = 0b010;
    // the head of the queue no longer points to this block
    const HEAD_ADVANCED: u8 = 0b100;
}

impl<T> Block<T> {
    fn alloc() -> *mut Block<T> {
        let block = unsafe { Box::into_raw(util::box_zeroed()) };
        assert_eq!(block as usize & Block::INDEX_MASK, 0);
        block
    }

    unsafe fn dealloc(block: *mut Block<T>) {
        let _ = Box::from_raw(block);
    }

    unsafe fn try_reclaim(block: *mut Block<T>, start: usize) {
        for slot in &block.deref().slots[start..] {
            if slot.state.load(Ordering::Acquire) & READ != READ {
                if slot.state.fetch_add(RESUME, Ordering::Relaxed) & READ != READ {
                    return;
                }
            }
        }

        let flags = block
            .deref()
            .reclamation
            .fetch_add(Block::CONSUMED, Ordering::AcqRel);

        if flags == Block::TAIL_ADVANCED | Block::HEAD_ADVANCED {
            let _ = Block::dealloc(block);
        }
    }

    unsafe fn try_reclaim_slow(
        block: *mut Block<T>,
        final_count: Option<u32>,
        increment_count: impl FnOnce(u32) -> u32,
        try_reclaim: impl FnOnce() -> bool,
    ) {
        let mask = match final_count {
            Some(final_count) => (final_count << Block::FINAL_COUNT_SHIFT) + 1,
            None => 1,
        };

        let prev_mask = increment_count(mask);
        let curr_count = (prev_mask & Block::CURRENT_COUNT_MASK) + 1;

        if curr_count == final_count.unwrap_or(prev_mask >> Block::FINAL_COUNT_SHIFT) {
            if try_reclaim() {
                let _ = Block::dealloc(block);
            }
        }
    }

    unsafe fn try_reclaim_head_slow(block: *mut Block<T>, final_count: Option<u32>) {
        Block::try_reclaim_slow(
            block,
            final_count,
            |mask| block.deref().pop_count.fetch_add(mask, Ordering::Relaxed),
            || {
                block
                    .deref()
                    .reclamation
                    .fetch_add(Block::HEAD_ADVANCED, Ordering::AcqRel)
                    == Block::CONSUMED | Block::TAIL_ADVANCED
            },
        );
    }

    unsafe fn try_reclaim_tail_slow(block: *mut Block<T>, final_count: Option<u32>) {
        Block::try_reclaim_slow(
            block,
            final_count,
            |mask| block.deref().push_count.fetch_add(mask, Ordering::Relaxed),
            || {
                block
                    .deref()
                    .reclamation
                    .fetch_add(Block::TAIL_ADVANCED, Ordering::AcqRel)
                    == Block::CONSUMED | Block::HEAD_ADVANCED
            },
        )
    }

    fn compare_exchange(
        block_ptr: &AtomicPtr<Block<T>>,
        mut current_tagged: *mut Block<T>,
        new: *mut Block<T>,
        current: *mut Block<T>,
    ) -> Option<u32> {
        while let Err(found) =
            block_ptr.compare_exchange(current_tagged, new, Ordering::Release, Ordering::Relaxed)
        {
            // the pointer itself changed
            if found.map_addr(|addr| addr & !Block::INDEX_MASK) != current {
                return None;
            }

            // the tag changed
            current_tagged = found;
        }

        Some(((current_tagged.addr() & Block::INDEX_MASK) - Block::SLOTS) as _)
    }
}

struct Slot<T> {
    value: UnsafeCell<MaybeUninit<T>>,
    state: AtomicU8,
}

// the slot is unitialized
const UNINIT: u8 = 0b0000;

// an element was written to the slot
const WRITTEN: u8 = 0b0001;

// a reader attempted to read the slot and found it
// uninitialized; the slot is invalid for further use.
//
// note that CONSUMED still needs to be set to complete
// the invalidation
const INVALID: u8 = 0b1000;

// the slot was consumed by a reader.
//
// this means that there was either a successful
// or failed read attempt of this slot; the slot
// will no longer be accessed
const CONSUMED: u8 = 0b0100;

// the value was successfully read from this slot
const READ: u8 = WRITTEN | CONSUMED;

// indicates that a future reader/writer should
// resume reclamation of subsequent slots
const RESUME: u8 = 0b0010;

// the slot was invalidated by a reader and the
// writer must resume reclaming the block
const WRITER_RESUME: u8 = INVALID | CONSUMED | RESUME;

// a reader must resume reclaming the block
const READER_RESUME: u8 = WRITTEN | RESUME;
