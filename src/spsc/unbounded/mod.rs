mod queue;

use crate::error::*;
use crate::wait::mpsc::WaitCell;
use crate::{blocking, rc, util};

use std::task::{Context, Poll};
use std::time::Duration;

pub(super) fn new<T>() -> (UnboundedSender<T>, UnboundedReceiver<T>) {
    let (tx, rx) = rc::alloc(Channel {
        queue: queue::Queue::new(),
        receiver: WaitCell::new(),
    });

    (UnboundedSender(tx), UnboundedReceiver(rx))
}

struct Channel<T> {
    queue: queue::Queue<T>,
    receiver: WaitCell,
}

pub struct UnboundedSender<T>(rc::Sender<Channel<T>, 1>);

unsafe impl<T: Send> Send for UnboundedSender<T> {}

impl<T> UnboundedSender<T> {
    pub fn send(&self, value: T) -> Result<(), SendError<T>> {
        if self.0.is_disconnected() {
            return Err(SendError(value));
        }

        unsafe { self.0.queue.push(value) }
        self.0.receiver.wake();
        Ok(())
    }
}

pub struct UnboundedReceiver<T>(rc::Receiver<Channel<T>, 1>);

unsafe impl<T: Send> Send for UnboundedReceiver<T> {}

impl<T> UnboundedReceiver<T> {
    pub fn try_recv(&self) -> Result<T, TryRecvError> {
        match unsafe { self.0.queue.pop() } {
            Some(value) => Ok(value),
            None if self.0.is_disconnected() => Err(TryRecvError::Disconnected),
            None => Err(TryRecvError::Empty),
        }
    }

    pub async fn recv(&self) -> Result<T, RecvError> {
        util::poll_fn(|cx| self.poll_recv(cx)).await
    }

    pub fn poll_recv(&self, cx: &mut Context<'_>) -> Poll<Result<T, RecvError>> {
        self.0.receiver.poll_fn(cx, || match self.try_recv() {
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
        unsafe { self.0.drop(|| self.0.receiver.wake()) }
    }
}

impl<T> Drop for UnboundedReceiver<T> {
    fn drop(&mut self) {
        unsafe { self.0.drop(|| {}) }
    }
}
