mod bounded;
mod unbounded;

use crate::error::*;
use crate::raw::parking::queue::{self, TaskQueue};
use crate::raw::{blocking, rc};

use std::task::Poll;
use std::time::Duration;

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

pub struct Sender<T>(rc::Sender<Channel<T>>);

unsafe impl<T: Send> Send for Sender<T> {}
unsafe impl<T: Send> Sync for Sender<T> {}

impl<T> Sender<T> {
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

    pub async fn send(&self, value: T) -> Result<(), SendError<T>> {
        let mut state = Some(value);
        self.send_inner(&mut state).await
    }

    pub fn send_blocking(&self, value: T) -> Result<(), SendError<T>> {
        unsafe { blocking::block_on(self.send(value)) }
    }

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

    pub async fn send_inner(&self, state: &mut Option<T>) -> Result<(), SendError<T>> {
        queue::block_on!(self.0.senders => {
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
            should_park: || self.0.queue.is_full()
        });
    }

    pub fn is_empty(&self) -> bool {
        self.0.queue.is_empty()
    }
}

pub struct Receiver<T>(rc::Receiver<Channel<T>>);

unsafe impl<T: Send> Send for Receiver<T> {}
unsafe impl<T: Send> Sync for Receiver<T> {}

impl<T> Receiver<T> {
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

    pub async fn recv(&self) -> Result<T, RecvError> {
        queue::block_on!(self.0.receivers => {
            poll: || match self.try_recv() {
                Ok(value) => Poll::Ready(Ok(value)),
                Err(TryRecvError::Disconnected) => Poll::Ready(Err(RecvError)),
                Err(TryRecvError::Empty) => Poll::Pending,
            },
            should_park: || self.0.queue.is_empty()
        })
    }

    pub fn recv_blocking(&self) -> Result<T, RecvError> {
        unsafe { blocking::block_on(self.recv()) }
    }

    pub fn recv_blocking_timeout(&self, timeout: Duration) -> Result<T, RecvTimeoutError> {
        match unsafe { blocking::block_on_timeout(self.recv(), timeout) } {
            Some(value) => value.map_err(RecvError::into),
            None => Err(RecvTimeoutError::Timeout),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.0.queue.is_empty()
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

pub struct UnboundedSender<T>(rc::Sender<UnboundedChannel<T>, { unbounded::MAX_SENDERS }>);

unsafe impl<T: Send> Send for UnboundedSender<T> {}
unsafe impl<T: Send> Sync for UnboundedSender<T> {}

impl<T> UnboundedSender<T> {
    pub fn send(&self, value: T) -> Result<(), SendError<T>> {
        if self.0.is_disconnected() {
            return Err(SendError(value));
        }

        self.0.queue.push(value);
        self.0.receivers.unpark_one();
        Ok(())
    }

    pub fn is_empty(&self) -> bool {
        self.0.queue.is_empty()
    }
}

pub struct UnboundedReceiver<T>(rc::Receiver<UnboundedChannel<T>, { unbounded::MAX_RECEIVERS }>);

unsafe impl<T: Send> Send for UnboundedReceiver<T> {}
unsafe impl<T: Send> Sync for UnboundedReceiver<T> {}

impl<T> UnboundedReceiver<T> {
    pub fn try_recv(&self) -> Result<T, TryRecvError> {
        match self.0.queue.pop() {
            Some(value) => Ok(value),
            None if self.0.is_disconnected() => {
                self.0.queue.pop().ok_or(TryRecvError::Disconnected)
            }
            None => Err(TryRecvError::Empty),
        }
    }

    pub async fn recv(&self) -> Result<T, RecvError> {
        queue::block_on!(self.0.receivers => {
            poll: || match self.try_recv() {
                Ok(value) => Poll::Ready(Ok(value)),
                Err(TryRecvError::Disconnected) => Poll::Ready(Err(RecvError)),
                Err(TryRecvError::Empty) => Poll::Pending,
            },
            should_park: || self.0.queue.is_empty()
        })
    }

    pub fn recv_blocking(&self) -> Result<T, RecvError> {
        unsafe { blocking::block_on(self.recv()) }
    }

    pub fn recv_blocking_timeout(&self, timeout: Duration) -> Result<T, RecvTimeoutError> {
        match unsafe { blocking::block_on_timeout(self.recv(), timeout) } {
            Some(value) => value.map_err(RecvError::into),
            None => Err(RecvTimeoutError::Timeout),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.0.queue.is_empty()
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
