mod queue;

use crate::error::*;
use crate::wait::WaitCell;
use crate::{blocking, rc};

use std::future::Future;
use std::hint;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

pub(super) fn new<T>() -> (UnboundedSender<T>, UnboundedReceiver<T>) {
    let channel = Channel {
        queue: queue::Queue::new(),
        receiver: WaitCell::new(),
    };

    let (tx, rx) = rc::alloc(channel);
    (UnboundedSender(tx), UnboundedReceiver(rx))
}

struct Channel<T> {
    queue: queue::Queue<T>,
    receiver: WaitCell,
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
        self.0.receiver.wake();
        Ok(())
    }
}

pub struct UnboundedReceiver<T>(rc::Receiver<Channel<T>, 1>);

unsafe impl<T: Send> Send for UnboundedReceiver<T> {}
// impl<T> !Sync for Receiver<T> {}

impl<T> UnboundedReceiver<T> {
    pub fn try_recv(&self) -> Result<T, TryRecvError> {
        match unsafe { self.0.queue.pop() } {
            Some(value) => Ok(value),
            None if self.0.is_disconnected() => Err(TryRecvError::Disconnected),
            None => Err(TryRecvError::Empty),
        }
    }

    pub async fn recv(&self) -> Result<T, RecvError> {
        struct RecvFuture<'a, T>(&'a UnboundedReceiver<T>);

        impl<'a, T> Future for RecvFuture<'a, T> {
            type Output = Result<T, RecvError>;

            fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
                self.0.poll_recv(cx)
            }
        }

        RecvFuture(self).await
    }

    pub fn poll_recv(&self, cx: &mut Context<'_>) -> Poll<Result<T, RecvError>> {
        loop {
            match unsafe { self.0.queue.pop() } {
                Some(value) => return Poll::Ready(Ok(value)),
                _ if self.0.is_disconnected() => return Poll::Ready(Err(RecvError)),
                _ => {}
            }

            match unsafe { self.0.receiver.poll(cx.waker()) } {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(_) => hint::spin_loop(),
            }
        }
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
        unsafe { self.0.queue.is_empty() }
    }
}

impl<T> Clone for UnboundedSender<T> {
    fn clone(&self) -> Self {
        UnboundedSender(self.0.clone())
    }
}

impl<T> Drop for UnboundedSender<T> {
    fn drop(&mut self) {
        unsafe {
            self.0.drop(|| {
                self.0.receiver.wake();
            })
        }
    }
}

impl<T> Drop for UnboundedReceiver<T> {
    fn drop(&mut self) {
        unsafe { self.0.drop(|| {}) }
    }
}
