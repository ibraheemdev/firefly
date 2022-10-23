//! Multi-producer single-consumer channels.
//!
//! The `Sender` half of a MPSC channel can be cloned and shared across multiple tasks.
//! These channels are useful when following a *fan-in* pattern, where multiple tasks
//! send data to a single worker.
//!
//! See the [crate documentation](crate) for details about channel usage in general.
//!
//! # Examples
//!
//! ```
//! # use tokio::task;
//! use firefly::mpsc;
//!
//! # #[tokio::main] async fn main() {
//! // create a bounded channel
//! let (tx, mut rx) = mpsc::bounded(4);
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
//! // wait for each message to be sent
//! while let Ok(i) = rx.recv().await {
//!     println!("{i}");
//! }
//! # }
//! ```

mod bounded;
mod unbounded;

use crate::docs::docs;
use crate::error::*;
use crate::raw::parking::queue::{self, TaskQueue};
use crate::raw::parking::task::{self, Task};
use crate::raw::rc;

use std::task::Poll;
use std::time::Duration;

#[doc = docs!(mpsc::bounded)]
pub fn bounded<T>(capacity: usize) -> (Sender<T>, Receiver<T>) {
    let (tx, rx) = rc::alloc(Channel {
        queue: bounded::Queue::new(capacity),
        receiver: Task::new(),
        senders: TaskQueue::new(),
    });

    (Sender(tx), Receiver(rx))
}

struct Channel<T> {
    queue: bounded::Queue<T>,
    receiver: Task,
    senders: TaskQueue,
}

/// The sending half of an MPSC channel.
///
/// This type can be cloned and shared across multiple tasks.
pub struct Sender<T>(rc::Sender<Channel<T>>);

unsafe impl<T: Send> Send for Sender<T> {}
unsafe impl<T: Send> Sync for Sender<T> {}

impl<T> Sender<T> {
    #[doc = docs!(mpsc::bounded::try_send)]
    pub fn try_send(&self, value: T) -> Result<(), TrySendError<T>> {
        if self.0.is_disconnected() {
            return Err(TrySendError::Disconnected(value));
        }

        self.0
            .queue
            .push(value)
            .map(|_| self.0.receiver.unpark())
            .map_err(TrySendError::Full)
    }

    #[doc = docs!(mpsc::bounded::send)]
    pub async fn send(&self, value: T) -> Result<(), SendError<T>> {
        let mut state = Some(value);
        queue::await_on!(self.0.senders => {
            poll: || self.poll_send(&mut state),
            unpark: || { self.0.queue.can_push() || self.0.is_disconnected() }
        })
    }

    #[doc = docs!(mpsc::bounded::send_blocking)]
    pub fn send_blocking(&self, value: T) -> Result<(), SendError<T>> {
        let mut state = Some(value);
        queue::block_on!(self.0.senders => {
            poll: || self.poll_send(&mut state),
            unpark: || { self.0.queue.can_push() || self.0.is_disconnected() }
        })
    }

    #[doc = docs!(mpsc::bounded::send_blocking_timeout)]
    pub fn send_blocking_timeout(&self, value: T, timeout: Duration) -> Result<(), SendTimeoutError<T>> {
        let mut state = Some(value);

        queue::block_on!(timeout, self.0.senders => {
            poll: || self.poll_send(&mut state),
            unpark: || { self.0.queue.can_push() || self.0.is_disconnected() }
        })
        .ok_or_else(|| SendTimeoutError::Timeout(state.take().unwrap()))
        .and_then(|val| val.map_err(SendError::into))
    }

    fn poll_send(&self, state: &mut Option<T>) -> Poll<Result<(), SendError<T>>> {
        let value = state.take().unwrap();
        match self.try_send(value) {
            Ok(()) => Poll::Ready(Ok(())),
            Err(TrySendError::Disconnected(value)) => Poll::Ready(Err(SendError(value))),
            Err(TrySendError::Full(value)) => {
                *state = Some(value);
                Poll::Pending
            }
        }
    }
}

/// The receiving half of an MPSC channel.
pub struct Receiver<T>(rc::Receiver<Channel<T>, 1>);

unsafe impl<T: Send> Send for Receiver<T> {}
unsafe impl<T: Send> Sync for Receiver<T> {}

impl<T> Receiver<T> {
    #[doc = docs!(mpsc::bounded::try_recv)]
    pub fn try_recv(&mut self) -> Result<T, TryRecvError> {
        match unsafe { self.0.queue.pop() } {
            Some(value) => {
                self.0.senders.unpark_one();
                Ok(value)
            }
            None if self.0.is_disconnected() => match unsafe { self.0.queue.pop() } {
                Some(value) => {
                    self.0.senders.unpark_one();
                    Ok(value)
                }
                _ => Err(TryRecvError::Disconnected),
            },
            None => Err(TryRecvError::Empty),
        }
    }

    #[doc = docs!(mpsc::bounded::recv)]
    pub async fn recv(&mut self) -> Result<T, RecvError> {
        task::await_on!(self.0.receiver => || self.poll_recv())
    }

    #[doc = docs!(mpsc::bounded::recv_blocking)]
    pub fn recv_blocking(&mut self) -> Result<T, RecvError> {
        task::block_on!(self.0.receiver => || self.poll_recv())
    }

    #[doc = docs!(mpsc::bounded::recv_blocking_timeout)]
    pub fn recv_blocking_timeout(&mut self, timeout: Duration) -> Result<T, RecvTimeoutError> {
        task::block_on!(timeout, self.0.receiver => || self.poll_recv())
            .ok_or(RecvTimeoutError::Timeout)
            .and_then(|val| val.map_err(RecvError::into))
    }

    fn poll_recv(&mut self) -> Poll<Result<T, RecvError>> {
        match self.try_recv() {
            Ok(value) => return Poll::Ready(Ok(value)),
            Err(TryRecvError::Disconnected) => return Poll::Ready(Err(RecvError)),
            Err(TryRecvError::Empty) => Poll::Pending,
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
        unsafe { self.0.drop(|| self.0.receiver.unpark()) }
    }
}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        unsafe { self.0.drop(|| self.0.senders.unpark_all()) }
    }
}

#[doc = docs!(mpsc::unbounded)]
pub fn unbounded<T>() -> (UnboundedSender<T>, UnboundedReceiver<T>) {
    let (tx, rx) = rc::alloc(UnboundedChannel {
        queue: unbounded::Queue::new(),
        receiver: Task::new(),
    });

    (UnboundedSender(tx), UnboundedReceiver(rx))
}

struct UnboundedChannel<T> {
    queue: unbounded::Queue<T>,
    receiver: Task,
}

/// The sending half of an unbounded MPSC channel.
///
/// This type can be cloned and shared across multiple tasks.
pub struct UnboundedSender<T>(rc::Sender<UnboundedChannel<T>, { unbounded::MAX_SENDERS }>);

unsafe impl<T: Send> Send for UnboundedSender<T> {}
unsafe impl<T: Send> Sync for UnboundedSender<T> {}

impl<T> UnboundedSender<T> {
    #[doc = docs!(mpsc::unbounded::send)]
    pub fn send(&self, value: T) -> Result<(), SendError<T>> {
        if self.0.is_disconnected() {
            return Err(SendError(value));
        }

        self.0.queue.push(value);
        self.0.receiver.unpark();
        Ok(())
    }
}

/// The receiving half of an unbounded MPSC channel.
pub struct UnboundedReceiver<T>(rc::Receiver<UnboundedChannel<T>, 1>);

unsafe impl<T: Send> Send for UnboundedReceiver<T> {}
unsafe impl<T: Send> Sync for UnboundedReceiver<T> {}

impl<T> UnboundedReceiver<T> {
    #[doc = docs!(mpsc::unbounded::try_recv)]
    pub fn try_recv(&mut self) -> Result<T, TryRecvError> {
        match unsafe { self.0.queue.pop() } {
            Some(value) => Ok(value),
            None if self.0.is_disconnected() => unsafe { self.0.queue.pop() }.ok_or(TryRecvError::Disconnected),
            None => Err(TryRecvError::Empty),
        }
    }

    #[doc = docs!(mpsc::unbounded::recv)]
    pub async fn recv(&mut self) -> Result<T, RecvError> {
        task::await_on!(self.0.receiver => || self.poll_recv())
    }

    #[doc = docs!(mpsc::unbounded::recv_blocking)]
    pub fn recv_blocking(&mut self) -> Result<T, RecvError> {
        task::block_on!(self.0.receiver => || self.poll_recv())
    }

    #[doc = docs!(mpsc::bounded::recv_blocking_timeout)]
    pub fn recv_blocking_timeout(&mut self, timeout: Duration) -> Result<T, RecvTimeoutError> {
        task::block_on!(timeout, self.0.receiver => || self.poll_recv())
            .ok_or(RecvTimeoutError::Timeout)
            .and_then(|val| val.map_err(RecvError::into))
    }

    fn poll_recv(&mut self) -> Poll<Result<T, RecvError>> {
        match self.try_recv() {
            Ok(value) => return Poll::Ready(Ok(value)),
            Err(TryRecvError::Disconnected) => return Poll::Ready(Err(RecvError)),
            Err(TryRecvError::Empty) => Poll::Pending,
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
        unsafe { self.0.drop(|| self.0.receiver.unpark()) }
    }
}

impl<T> Drop for UnboundedReceiver<T> {
    fn drop(&mut self) {
        unsafe { self.0.drop(|| {}) }
    }
}
