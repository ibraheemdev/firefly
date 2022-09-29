use crate::error::*;
use crate::raw::parking::task::{self, Task};
use crate::raw::queues::spsc_unbounded as queue;
use crate::raw::{blocking, rc};

use std::task::Poll;
use std::time::Duration;

pub(super) fn new<T>() -> (UnboundedSender<T>, UnboundedReceiver<T>) {
    let (tx, rx) = rc::alloc(Channel {
        queue: queue::Queue::new(),
        receiver: Task::new(),
    });

    (UnboundedSender(tx), UnboundedReceiver(rx))
}

struct Channel<T> {
    queue: queue::Queue<T>,
    receiver: Task,
}

pub struct UnboundedSender<T>(rc::Sender<Channel<T>, 1>);

unsafe impl<T: Send> Send for UnboundedSender<T> {}

impl<T> UnboundedSender<T> {
    pub fn send(&self, value: T) -> Result<(), SendError<T>> {
        if self.0.is_disconnected() {
            return Err(SendError(value));
        }

        unsafe { self.0.queue.push(value) }
        self.0.receiver.unpark();
        Ok(())
    }
}

pub struct UnboundedReceiver<T>(rc::Receiver<Channel<T>, 1>);

unsafe impl<T: Send> Send for UnboundedReceiver<T> {}

impl<T> UnboundedReceiver<T> {
    pub fn try_recv(&self) -> Result<T, TryRecvError> {
        match unsafe { self.0.queue.pop() } {
            Some(value) => Ok(value),
            None if self.0.is_disconnected() => {
                unsafe { self.0.queue.pop() }.ok_or(TryRecvError::Disconnected)
            }
            None => Err(TryRecvError::Empty),
        }
    }

    pub async fn recv(&self) -> Result<T, RecvError> {
        task::block_on!(self.0.receiver => || match self.try_recv() {
            Ok(value) => return Poll::Ready(Ok(value)),
            Err(TryRecvError::Disconnected) => return Poll::Ready(Err(RecvError)),
            Err(TryRecvError::Empty) => Poll::Pending,
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
