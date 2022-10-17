use super::util::UnsafeDeref;

use std::ops::Deref;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// Allocate a channel as a sender/receiver pair.
pub fn alloc<T, const SENDERS: usize, const RECEIVERS: usize>(
    chan: T,
) -> (Sender<T, SENDERS>, Receiver<T, RECEIVERS>) {
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

/// A reference counted channel.
struct Rc<T> {
    senders: AtomicUsize,
    receivers: AtomicUsize,
    disconnected: AtomicBool,
    chan: T,
}

/// The sending half of a channel.
pub struct Sender<T, const MAX: usize = { isize::MAX as _ }> {
    rc: *mut Rc<T>,
}

unsafe impl<T: Send, const MAX: usize> Send for Sender<T, MAX> {}
unsafe impl<T: Sync, const MAX: usize> Sync for Sender<T, MAX> {}

impl<T, const MAX: usize> Sender<T, MAX> {
    /// Clones this sender.
    pub fn clone(&self) -> Sender<T, MAX> {
        let count = unsafe { self.rc.deref().senders.fetch_add(1, Ordering::Relaxed) };

        if count > MAX {
            panic!("exceeded max sender count of {}", MAX);
        }

        Sender { rc: self.rc }
    }

    /// Drops the sender, running `disconnect` if this is the last sender handle
    /// and the receiver is still connected.
    pub unsafe fn drop<F>(&self, disconnect: F)
    where
        F: FnOnce(),
    {
        if self.rc.deref().senders.fetch_sub(1, Ordering::AcqRel) == 1 {
            let disconnected = self.rc.deref().disconnected.swap(true, Ordering::AcqRel);

            if disconnected {
                let _ = Box::from_raw(self.rc);
            } else {
                disconnect()
            }
        }
    }

    ///  Returns `true` if the receiving half of the channel is disconnected.
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

/// The receiving half of a channel.
pub struct Receiver<T, const MAX: usize = { isize::MAX as _ }> {
    rc: *mut Rc<T>,
}

unsafe impl<T: Send, const MAX: usize> Send for Receiver<T, MAX> {}
unsafe impl<T: Sync, const MAX: usize> Sync for Receiver<T, MAX> {}

impl<T, const MAX: usize> Receiver<T, MAX> {
    /// Clones this receiver.
    pub fn clone(&self) -> Receiver<T, MAX> {
        let count = unsafe { self.rc.deref().receivers.fetch_add(1, Ordering::Relaxed) };

        if count > MAX {
            panic!("exceeded max receiver count of {}", MAX);
        }

        Receiver { rc: self.rc }
    }

    /// Drops the receiver, running `disconnect` if this is the last receiver handle
    /// and the sender is still connected.
    pub unsafe fn drop<F>(&self, disconnect: F)
    where
        F: FnOnce(),
    {
        if self.rc.deref().receivers.fetch_sub(1, Ordering::AcqRel) == 1 {
            let disconnected = self.rc.deref().disconnected.swap(true, Ordering::AcqRel);

            if disconnected {
                let _ = Box::from_raw(self.rc);
            } else {
                disconnect()
            }
        }
    }

    ///  Returns `true` if the receiving half of the channel is disconnected.
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
