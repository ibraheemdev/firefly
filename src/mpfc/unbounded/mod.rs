mod queue;

use crate::error::*;
use crate::wait::mpmc::WaitQueue;
use crate::{blocking, rc};

use std::task::Poll;
use std::time::Duration;

pub(super) fn new<T>() -> (UnboundedSender<T>, UnboundedReceiver<T>) {
    let (tx, rx) = rc::alloc(Channel {
        queue: queue::Queue::new(),
        receivers: WaitQueue::new(),
    });

    (UnboundedSender(tx), UnboundedReceiver(rx))
}

struct Channel<T> {
    queue: queue::Queue<T>,
    receivers: WaitQueue,
}

pub struct UnboundedSender<T>(rc::Sender<Channel<T>, { queue::MAX_SENDERS }>);

unsafe impl<T: Send> Send for UnboundedSender<T> {}
unsafe impl<T: Send> Sync for UnboundedSender<T> {}

impl<T> UnboundedSender<T> {
    pub fn send(&self, value: T) -> Result<(), SendError<T>> {
        if self.0.is_disconnected() {
            return Err(SendError(value));
        }

        self.0.queue.push(value);
        self.0.receivers.wake();
        Ok(())
    }

    pub fn is_empty(&self) -> bool {
        self.0.queue.is_empty()
    }
}

pub struct UnboundedReceiver<T>(rc::Receiver<Channel<T>, { queue::MAX_RECEIVERS }>);

unsafe impl<T: Send> Send for UnboundedReceiver<T> {}
unsafe impl<T: Send> Sync for UnboundedReceiver<T> {}

impl<T> UnboundedReceiver<T> {
    pub fn try_recv(&self) -> Result<T, TryRecvError> {
        match self.0.queue.pop() {
            Some(value) => Ok(value),
            None if self.0.is_disconnected() => Err(TryRecvError::Disconnected),
            None => Err(TryRecvError::Empty),
        }
    }

    pub async fn recv(&self) -> Result<T, RecvError> {
        self.0
            .receivers
            .poll_fn(
                || self.0.queue.is_empty(),
                || match self.try_recv() {
                    Ok(value) => Poll::Ready(Ok(value)),
                    Err(TryRecvError::Disconnected) => Poll::Ready(Err(RecvError)),
                    Err(TryRecvError::Empty) => Poll::Pending,
                },
            )
            .await
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
        unsafe { self.0.drop(|| self.0.receivers.wake_all()) }
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
