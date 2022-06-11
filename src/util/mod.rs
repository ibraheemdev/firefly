pub mod intrusive_list;

use std::alloc::{self, Layout};
use std::ptr;
use std::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};
use std::task::{RawWaker, RawWakerVTable, Waker};

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

pub unsafe fn box_zeroed<T>() -> Box<T> {
    let layout = Layout::new::<T>();

    let ptr = alloc::alloc_zeroed(layout);

    if ptr.is_null() {
        alloc::handle_alloc_error(layout);
    }

    Box::from_raw(ptr as *mut T)
}

pub trait UnsafeDeref<T> {
    unsafe fn deref<'a>(self) -> &'a T;
    unsafe fn deref_mut<'a>(self) -> &'a mut T;
}

impl<T> UnsafeDeref<T> for *mut T {
    unsafe fn deref<'a>(self) -> &'a T {
        &*self
    }

    unsafe fn deref_mut<'a>(self) -> &'a mut T {
        &mut *self
    }
}

// Pads and aligns a value to the length of two cache lines.
#[cfg_attr(
    any(
        target_arch = "x86_64",
        target_arch = "aarch64",
        target_arch = "powerpc64",
    ),
    repr(align(256))
)]
#[cfg_attr(
    any(
        target_arch = "arm",
        target_arch = "mips",
        target_arch = "mips64",
        target_arch = "riscv64",
    ),
    repr(align(64))
)]
#[cfg_attr(target_arch = "s390x", repr(align(512)))]
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
    repr(align(128))
)]
#[derive(Default)]
pub struct DoubleCachePadded<T>(pub T);

impl<T> std::ops::Deref for DoubleCachePadded<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T> std::ops::DerefMut for DoubleCachePadded<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.0
    }
}

pub trait FetchAddPtr<T> {
    fn fetch_add(&self, n: usize, ordering: Ordering) -> *mut T;
}

impl<T> FetchAddPtr<T> for AtomicPtr<T> {
    fn fetch_add(&self, n: usize, ordering: Ordering) -> *mut T {
        let raw = unsafe { &*(self as *const _ as *const AtomicUsize) };
        raw.fetch_add(n, ordering) as _
    }
}

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
