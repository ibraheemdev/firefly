use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};

/// A wrapper around an atomic integer with optimized `Acquire/Release` loads
/// for use as a signal.
#[derive(Default)]
#[repr(transparent)]
pub struct Signal<T>(T);

/// Ordering used on any stores to be released by the signal,
/// i.e. `queue.push`, and `bounded_queue.pop`.
pub const RELEASE: Ordering = Ordering::SeqCst;

impl<T: AtomicInt> Signal<T> {
    pub fn new(x: impl Into<T>) -> Signal<T> {
        Signal(x.into())
    }

    /// Performs a 'release load', equivalent to a full fence followed
    /// by a load.
    ///
    /// This operation is cheaper than a full fence, but only works if
    /// the store being released was made with `Atomicparking::RELEASE` ordering.
    pub fn load_release(&self) -> T::Inner {
        if !cfg!(miri) && cfg!(any(target_arch = "x86_64", target_arch = "aarch64")) {
            // `store(SeqCst); load(SeqCst);` is a store-load barrier that acts
            // as a full fence on ARM and x86 (unsure about other architectures)
            //
            // - ARM: `stlr; ldar`, explicitly guarantees store-load barrier
            // - x86: `xchg; mov`, xchg is a full barrier
            return self.0.load(Ordering::SeqCst);
        }

        self.0.fetch_add_zero(Ordering::Release)
    }

    /// Loads the value with acquire ordering.
    pub fn load_acquire(&self) -> T::Inner {
        if !cfg!(miri) && cfg!(any(target_arch = "x86_64", target_arch = "aarch64")) {
            // `load_release` provides the necessary barrier, nothing special needed here
            return self.0.load(Ordering::Acquire);
        }

        self.0.fetch_add_zero(Ordering::Acquire)
    }

    pub fn inner(&self) -> &T {
        &self.0
    }
}

pub trait AtomicInt {
    type Inner;
    fn load(&self, ordering: Ordering) -> Self::Inner;
    fn fetch_add_zero(&self, ordering: Ordering) -> Self::Inner;
}

impl AtomicInt for AtomicU8 {
    type Inner = u8;

    fn load(&self, ordering: Ordering) -> u8 {
        self.load(ordering)
    }

    fn fetch_add_zero(&self, ordering: Ordering) -> u8 {
        self.fetch_add(0, ordering)
    }
}

impl AtomicInt for AtomicUsize {
    type Inner = usize;

    fn load(&self, ordering: Ordering) -> usize {
        self.load(ordering)
    }

    fn fetch_add_zero(&self, ordering: Ordering) -> usize {
        self.fetch_add(0, ordering)
    }
}
