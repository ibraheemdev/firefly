use crate::util::UnsafeDeref;

use std::ops::Deref;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

pub fn alloc<C, const MAX_SENDERS: usize, const MAX_RECEIVERS: usize>(
    chan: C,
) -> (Sender<C, MAX_SENDERS>, Receiver<C, MAX_RECEIVERS>) {
    let rc = Box::into_raw(Box::new(Rc {
        senders: AtomicUsize::new(1),
        receivers: AtomicUsize::new(1),
        disconnected: AtomicBool::new(false),
        chan,
    }));

    let tx = Sender { rc };
    let rx = Receiver { rc };
    (tx, rx)
}

struct Rc<T> {
    senders: AtomicUsize,
    receivers: AtomicUsize,
    disconnected: AtomicBool,
    chan: T,
}

pub struct Sender<T, const MAX: usize = { isize::MAX as _ }> {
    rc: *mut Rc<T>,
}

impl<T, const MAX: usize> Sender<T, MAX> {
    pub fn clone(&self) -> Sender<T, MAX> {
        let count = unsafe { self.rc.deref().senders.fetch_add(1, Ordering::Relaxed) };

        if count > MAX {
            std::process::abort();
        }

        Sender { rc: self.rc }
    }

    pub unsafe fn drop<F>(&self, wake: F)
    where
        F: FnOnce(),
    {
        if self.rc.deref().senders.fetch_sub(1, Ordering::AcqRel) == 1 {
            let disconnected = self.rc.deref().disconnected.swap(true, Ordering::AcqRel);

            wake();

            if disconnected {
                let _ = Box::from_raw(self.rc);
            }
        }
    }

    pub fn is_disconnected(&self) -> bool {
        unsafe { self.rc.deref().disconnected.load(Ordering::Relaxed) }
    }
}

impl<T, const MAX: usize> Deref for Sender<T, MAX> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { &self.rc.deref().chan }
    }
}

pub struct Receiver<T, const MAX: usize = { isize::MAX as _ }> {
    rc: *mut Rc<T>,
}

impl<T, const MAX: usize> Receiver<T, MAX> {
    pub fn clone(&self) -> Receiver<T, MAX> {
        let count = unsafe { self.rc.deref().receivers.fetch_add(1, Ordering::Relaxed) };

        if count > MAX {
            std::process::abort();
        }

        Receiver { rc: self.rc }
    }

    pub unsafe fn drop<F>(&self, wake: F)
    where
        F: FnOnce(),
    {
        if self.rc.deref().receivers.fetch_sub(1, Ordering::AcqRel) == 1 {
            let disconnected = self.rc.deref().disconnected.swap(true, Ordering::AcqRel);

            wake();

            if disconnected {
                let _ = Box::from_raw(self.rc);
            }
        }
    }

    pub fn is_disconnected(&self) -> bool {
        unsafe { self.rc.deref().disconnected.load(Ordering::Relaxed) }
    }
}

impl<T, const MAX: usize> Deref for Receiver<T, MAX> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { &self.rc.deref().chan }
    }
}
