use std::alloc::{self, Layout};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};
use std::task::{Context, Poll};

pub struct UnsafeSend<T>(pub T);

unsafe impl<T> Send for UnsafeSend<T> {}

impl<T: Future> Future for UnsafeSend<T> {
    type Output = T::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // safety: pin projection
        unsafe { self.map_unchecked_mut(|x| &mut x.0).poll(cx) }
    }
}

pub fn assert_valid_capacity(capacity: usize) {
    if capacity == 0 {
        panic!("`capacity` cannot be zero")
    }

    if capacity & (capacity - 1) != 0 {
        panic!("`capacity` must be a power of two");
    }
}

/// Allocates a box with zeroed memory.
///
/// # Safety
///
/// `T` must be valid for the zero bit-pattern.
pub unsafe fn box_zeroed<T>() -> Box<T> {
    let layout = Layout::new::<T>();

    let ptr = alloc::alloc_zeroed(layout);

    if ptr.is_null() {
        alloc::handle_alloc_error(layout);
    }

    Box::from_raw(ptr as *mut T)
}

/// Postfix deref for pointers.
pub trait UnsafeDeref<T> {
    unsafe fn deref<'a>(self) -> &'a T;
    unsafe fn deref_mut<'a>(self) -> &'a mut T;
}

impl<T> UnsafeDeref<T> for *const T {
    unsafe fn deref<'a>(self) -> &'a T {
        &*self
    }

    unsafe fn deref_mut<'a>(self) -> &'a mut T {
        panic!("called deref_mut on *const T")
    }
}

impl<T> UnsafeDeref<T> for *mut T {
    unsafe fn deref<'a>(self) -> &'a T {
        &*self
    }

    unsafe fn deref_mut<'a>(self) -> &'a mut T {
        &mut *self
    }
}

/// Polyfill for the unstable `AtomicPtr::fetch_add`.
pub trait FetchAddPtr<T> {
    fn fetch_add(&self, n: usize, ordering: Ordering) -> *mut T;
}

impl<T> FetchAddPtr<T> for AtomicPtr<T> {
    fn fetch_add(&self, n: usize, ordering: Ordering) -> *mut T {
        let raw = unsafe { &*(self as *const _ as *const AtomicUsize) };
        raw.fetch_add(n, ordering) as _
    }
}

/// Polyfill for the unstable strict-provenance APIs.
pub unsafe trait StrictProvenance: Sized {
    fn addr(self) -> usize;
    fn with_addr(self, addr: usize) -> Self;
    fn map_addr(self, f: impl FnOnce(usize) -> usize) -> Self;
}

unsafe impl<T> StrictProvenance for *mut T {
    fn addr(self) -> usize {
        self as usize
    }

    fn with_addr(self, addr: usize) -> Self {
        addr as Self
    }

    fn map_addr(self, f: impl FnOnce(usize) -> usize) -> Self {
        self.with_addr(f(self.addr()))
    }
}

pub fn poll_fn<T, F>(poll: F) -> impl Future<Output = T>
where
    F: FnMut(&mut Context<'_>) -> Poll<T>,
{
    /// A future that wraps a `poll` function.
    struct PollFn<F>(F);

    impl<F> Unpin for PollFn<F> {}

    impl<T, F> Future for PollFn<F>
    where
        F: FnMut(&mut Context<'_>) -> Poll<T>,
    {
        type Output = T;

        fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<T> {
            (self.0)(cx)
        }
    }

    PollFn(poll)
}

/// Pads a value to the length of a cacheline.
#[cfg_attr(
    any(
        target_arch = "x86_64",
        target_arch = "aarch64",
        target_arch = "powerpc64",
    ),
    repr(align(128))
)]
#[cfg_attr(
    any(
        target_arch = "arm",
        target_arch = "mips",
        target_arch = "mips64",
        target_arch = "riscv64",
    ),
    repr(align(32))
)]
#[cfg_attr(target_arch = "s390x", repr(align(256)))]
#[cfg_attr(
    not(any(
        target_arch = "x86_64",
        target_arch = "aarch64",
        target_arch = "powerpc64",
        target_arch = "arm",
        target_arch = "mips",
        target_arch = "mips64",
        target_arch = "riscv64",
        target_arch = "s390x",
    )),
    repr(align(64))
)]
#[derive(Default)]
#[repr(C)]
pub struct CachePadded<T>(pub T);

impl<T> std::ops::Deref for CachePadded<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T> std::ops::DerefMut for CachePadded<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.0
    }
}
