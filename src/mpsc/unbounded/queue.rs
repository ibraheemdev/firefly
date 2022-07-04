use crate::util::{self, CachePadded, FetchAddPtr, StrictProvenance, UnsafeDeref};

use std::cell::{Cell, UnsafeCell};
use std::mem::{ManuallyDrop, MaybeUninit};
use std::ptr;
use std::sync::atomic::{AtomicPtr, AtomicU32, AtomicU8, Ordering};

pub const MAX_SENDERS: usize = (1 << Block::TAG_BITS) - Block::SLOTS + 1;
const SPIN_READ: usize = 16;

pub struct Queue<T> {
    head: CachePadded<Cell<(*mut Block<T>, usize)>>,
    tail: CachePadded<AtomicPtr<Block<T>>>,
    cached_tail: AtomicPtr<Block<T>>,
}

unsafe impl<T: Send> Send for Queue<T> {}

impl<T> Queue<T> {
    pub fn new() -> Queue<T> {
        let block = Block::alloc();

        Queue {
            head: CachePadded(Cell::new((block, 0))),
            tail: CachePadded(AtomicPtr::new(block)),
            cached_tail: AtomicPtr::new(block),
        }
    }

    pub unsafe fn pop(&self) -> Option<T> {
        loop {
            if self.is_empty() {
                return None;
            }

            let (head, index) = self.head.get();

            if index < Block::SLOTS {
                let slot = head.deref().slots.get_unchecked(index);

                for _ in 0..SPIN_READ {
                    if slot.state.load(Ordering::Acquire) & WRITTEN == WRITTEN {
                        let value = slot.value.get().read().assume_init();
                        let state = slot.state.fetch_add(CONSUMED, Ordering::Release);
                        assert_eq!(state, WRITTEN);
                        self.head.set((head, index + 1));
                        return Some(value);
                    }

                    std::hint::spin_loop();
                }

                match slot.state.fetch_add(INVALID, Ordering::Acquire) {
                    WRITTEN => {
                        let value = slot.value.get().read().assume_init();
                        slot.state.fetch_add(CONSUMED, Ordering::Release);
                        self.head.set((head, index + 1));
                        return Some(value);
                    }
                    state => {
                        assert!(state & RESUME == 0);
                        slot.state.fetch_add(CONSUMED, Ordering::Release);
                        self.head.set((head, index + 1));
                        continue;
                    }
                };
            }

            if index == Block::SLOTS {
                self.head.set((head, index + 1));
                Block::try_reclaim(head, 0);
            }

            let tail = self
                .tail
                .load(Ordering::Acquire)
                .map_addr(|addr| addr & !Block::INDEX_MASK);

            if head == tail {
                return None;
            }

            let next = head.deref().next.load(Ordering::Acquire);
            self.head.set((next, 0));

            let state = head
                .deref()
                .reclamation
                .fetch_add(Block::HEAD_ADVANCED, Ordering::AcqRel);

            if state == Block::CONSUMED | Block::TAIL_ADVANCED {
                let _ = Block::dealloc(head);
            }
        }
    }

    pub unsafe fn is_empty(&self) -> bool {
        let cached_tail = self.cached_tail.load(Ordering::Acquire);
        let (head, head_index) = self.head.get();

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

            let (head, _) = self.head.get();

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

    unsafe fn try_reclaim_tail_slow(block: *mut Block<T>, final_count: Option<u32>) {
        let mask = match final_count {
            Some(final_count) => (final_count << Block::FINAL_COUNT_SHIFT) + 1,
            None => 1,
        };

        let prev_mask = block.deref().push_count.fetch_add(mask, Ordering::Relaxed);

        let curr_count = (prev_mask & Block::CURRENT_COUNT_MASK) + 1;

        if curr_count == final_count.unwrap_or(prev_mask >> Block::FINAL_COUNT_SHIFT) {
            let flags = block
                .deref()
                .reclamation
                .fetch_add(Block::TAIL_ADVANCED, Ordering::AcqRel);

            if flags == Block::CONSUMED | Block::HEAD_ADVANCED {
                let _ = Block::dealloc(block);
            }
        }
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
