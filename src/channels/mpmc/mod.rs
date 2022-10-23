//! Multi-producer multi-consumer channels.
//!
//! Both the `Sender` and `Receiver` halves of a MPSC channel can be cloned and
//! shared across multiple tasks. These channels are useful when are not following
//! a traditional SPSC, SPMC, or MPSC pattern.
//!
//! See the [crate documentation](crate) for details about channel usage in general.
//!
//! # Examples
//!
//! ```
//! # use tokio::task;
//! use firefly::mpmc;
//!
//! # #[tokio::main] async fn main() {
//! // create a bounded channel
//! let (tx, rx) = mpmc::bounded(4);
//!
//! // spawn 4 tasks, each sending a single message
//! for i in 0..4 {
//!     let tx = tx.clone();
//!     task::spawn(async move {
//!         tx.send(i).await.unwrap();
//!     });
//! }
//!
//! // drop the last sender to stop `rx` from waiting for a message
//! drop(tx);
//!
//! // spawn 4 tasks, each receiving a single message
//! for i in 0..4 {
//!     let rx = rx.clone();
//!     task::spawn(async move {
//!         let i = rx.recv().await.unwrap();
//!         println!("{i}");
//!     });
//! }
//! # }
//! ```

mod bounded;
mod unbounded;

use crate::docs::docs;
use crate::error::*;
use crate::raw::parking::queue::{self, TaskQueue};
use crate::raw::{blocking, rc};

use std::task::Poll;
use std::time::Duration;

#[doc = docs!(mpmc::bounded)]
pub fn bounded<T>(capacity: usize) -> (Sender<T>, Receiver<T>) {
    let (tx, rx) = rc::alloc(Channel {
        queue: bounded::Queue::new(capacity),
        receivers: TaskQueue::new(),
        senders: TaskQueue::new(),
    });

    (Sender(tx), Receiver(rx))
}

struct Channel<T> {
    queue: bounded::Queue<T>,
    receivers: TaskQueue,
    senders: TaskQueue,
}

/// The sending half of an MPMC channel.
///
/// This type can be cloned and shared across multiple tasks.
pub struct Sender<T>(rc::Sender<Channel<T>>);

unsafe impl<T: Send> Send for Sender<T> {}
unsafe impl<T: Send> Sync for Sender<T> {}

impl<T> Sender<T> {
    #[doc = docs!(mpmc::bounded::try_send)]
    pub fn try_send(&self, value: T) -> Result<(), TrySendError<T>> {
        if self.0.is_disconnected() {
            return Err(TrySendError::Disconnected(value));
        }

        self.0
            .queue
            .push(value)
            .map(|_| self.0.receivers.unpark_one())
            .map_err(TrySendError::Full)
    }

    #[doc = docs!(mpmc::bounded::send)]
    pub async fn send(&self, value: T) -> Result<(), SendError<T>> {
        let mut state = Some(value);
        self.send_inner(&mut state).await
    }

    #[doc = docs!(mpmc::bounded::send_blocking)]
    pub fn send_blocking(&self, value: T) -> Result<(), SendError<T>> {
        unsafe { blocking::block_on(self.send(value)) }
    }

    #[doc = docs!(mpmc::bounded::send_blocking_timeout)]
    pub fn send_blocking_timeout(
        &self,
        value: T,
        timeout: Duration,
    ) -> Result<(), SendTimeoutError<T>> {
        let mut state = Some(value);

        match unsafe { blocking::block_on_timeout(self.send_inner(&mut state), timeout) } {
            Some(value) => value.map_err(SendError::into),
            None => Err(SendTimeoutError::Timeout(state.take().unwrap())),
        }
    }

    async fn send_inner(&self, state: &mut Option<T>) -> Result<(), SendError<T>> {
        queue::await_on!(self.0.senders => {
            poll: || {
                let value = state.take().unwrap();
                match self.try_send(value) {
                    Ok(()) => Poll::Ready(Ok(())),
                    Err(TrySendError::Disconnected(value)) => Poll::Ready(Err(SendError(value))),
                    Err(TrySendError::Full(value)) => {
                        *state = Some(value);
                        Poll::Pending
                    }
                }
            },
            unpark: || { self.0.queue.can_push() || self.0.is_disconnected() }
        })
    }
}

/// The receiving half of an MPMC channnel.
///
/// This type can be cloned and shared across multiple tasks.
pub struct Receiver<T>(rc::Receiver<Channel<T>>);

unsafe impl<T: Send> Send for Receiver<T> {}
unsafe impl<T: Send> Sync for Receiver<T> {}

impl<T> Receiver<T> {
    #[doc = docs!(mpmc::bounded::try_recv)]
    pub fn try_recv(&self) -> Result<T, TryRecvError> {
        match self.0.queue.pop() {
            Some(value) => {
                self.0.senders.unpark_one();
                Ok(value)
            }
            None if self.0.is_disconnected() => match self.0.queue.pop() {
                Some(value) => {
                    self.0.senders.unpark_one();
                    Ok(value)
                }
                _ => Err(TryRecvError::Disconnected),
            },
            None => Err(TryRecvError::Empty),
        }
    }

    #[doc = docs!(mpmc::bounded::recv)]
    pub async fn recv(&self) -> Result<T, RecvError> {
        queue::await_on!(self.0.receivers => {
            poll: || match self.try_recv() {
                Ok(value) => Poll::Ready(Ok(value)),
                Err(TryRecvError::Disconnected) => Poll::Ready(Err(RecvError)),
                Err(TryRecvError::Empty) => Poll::Pending,
            },
            unpark: || { self.0.queue.can_pop() || self.0.is_disconnected() }
        })
    }

    #[doc = docs!(mpmc::bounded::recv_blocking)]
    pub fn recv_blocking(&self) -> Result<T, RecvError> {
        unsafe { blocking::block_on(self.recv()) }
    }

    #[doc = docs!(mpmc::bounded::recv_blocking_timeout)]
    pub fn recv_blocking_timeout(&self, timeout: Duration) -> Result<T, RecvTimeoutError> {
        match unsafe { blocking::block_on_timeout(self.recv(), timeout) } {
            Some(value) => value.map_err(RecvError::into),
            None => Err(RecvTimeoutError::Timeout),
        }
    }
}

impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        Sender(self.0.clone())
    }
}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        unsafe { self.0.drop(|| self.0.receivers.unpark_all()) }
    }
}

impl<T> Clone for Receiver<T> {
    fn clone(&self) -> Self {
        Receiver(self.0.clone())
    }
}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        unsafe { self.0.drop(|| self.0.senders.unpark_all()) }
    }
}

#[doc = docs!(mpmc::unbounded)]
pub fn unbounded<T>() -> (UnboundedSender<T>, UnboundedReceiver<T>) {
    let (tx, rx) = rc::alloc(UnboundedChannel {
        queue: unbounded::Queue::new(),
        receivers: TaskQueue::new(),
    });

    (UnboundedSender(tx), UnboundedReceiver(rx))
}

struct UnboundedChannel<T> {
    queue: unbounded::Queue<T>,
    receivers: TaskQueue,
}

/// The sending half of an unbounded MPMC channnel.
///
/// This type can be cloned and shared across multiple tasks.
pub struct UnboundedSender<T>(rc::Sender<UnboundedChannel<T>, { unbounded::MAX_SENDERS }>);

unsafe impl<T: Send> Send for UnboundedSender<T> {}
unsafe impl<T: Send> Sync for UnboundedSender<T> {}

impl<T> UnboundedSender<T> {
    #[doc = docs!(mpmc::unbounded::send)]
    pub fn send(&self, value: T) -> Result<(), SendError<T>> {
        if self.0.is_disconnected() {
            return Err(SendError(value));
        }

        self.0.queue.push(value);
        self.0.receivers.unpark_one();
        Ok(())
    }
}

/// The receiving half of an unbounded MPMC channnel.
///
/// This type can be cloned and shared across multiple tasks.
pub struct UnboundedReceiver<T>(rc::Receiver<UnboundedChannel<T>, { unbounded::MAX_RECEIVERS }>);

unsafe impl<T: Send> Send for UnboundedReceiver<T> {}
unsafe impl<T: Send> Sync for UnboundedReceiver<T> {}

impl<T> UnboundedReceiver<T> {
    #[doc = docs!(mpmc::unbounded::try_recv)]
    pub fn try_recv(&self) -> Result<T, TryRecvError> {
        match self.0.queue.pop() {
            Some(value) => Ok(value),
            None if self.0.is_disconnected() => {
                self.0.queue.pop().ok_or(TryRecvError::Disconnected)
            }
            None => Err(TryRecvError::Empty),
        }
    }

    #[doc = docs!(mpmc::unbounded::recv)]
    pub async fn recv(&self) -> Result<T, RecvError> {
        queue::await_on!(self.0.receivers => {
            poll: || match self.try_recv() {
                Ok(value) => Poll::Ready(Ok(value)),
                Err(TryRecvError::Disconnected) => Poll::Ready(Err(RecvError)),
                Err(TryRecvError::Empty) => Poll::Pending,
            },
            unpark: || { !self.0.queue.is_empty() || self.0.is_disconnected() }
        })
    }

    #[doc = docs!(mpmc::unbounded::recv_blocking)]
    pub fn recv_blocking(&self) -> Result<T, RecvError> {
        unsafe { blocking::block_on(self.recv()) }
    }

    #[doc = docs!(mpmc::unbounded::recv_blocking_timeout)]
    pub fn recv_blocking_timeout(&self, timeout: Duration) -> Result<T, RecvTimeoutError> {
        match unsafe { blocking::block_on_timeout(self.recv(), timeout) } {
            Some(value) => value.map_err(RecvError::into),
            None => Err(RecvTimeoutError::Timeout),
        }
    }
}

impl<T> Clone for UnboundedSender<T> {
    fn clone(&self) -> Self {
        UnboundedSender(self.0.clone())
    }
}

impl<T> Drop for UnboundedSender<T> {
    fn drop(&mut self) {
        unsafe { self.0.drop(|| self.0.receivers.unpark_all()) }
    }
}

impl<T> Clone for UnboundedReceiver<T> {
    fn clone(&self) -> Self {
        UnboundedReceiver(self.0.clone())
    }
}

impl<T> Drop for UnboundedReceiver<T> {
    fn drop(&mut self) {
        unsafe { self.0.drop(|| {}) }
    }
}
