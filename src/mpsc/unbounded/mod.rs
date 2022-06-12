mod queue;

use crate::error::*;
use crate::wait::WaitCell;
use crate::{blocking, rc};

use std::future::Future;
use std::hint;
use std::pin::Pin;
use std::task::{Context, Poll};

pub(super) fn new<T>() -> (Sender<T>, Receiver<T>) {
    let channel = Channel {
        queue: queue::Queue::new(),
        receiver: WaitCell::new(),
    };

    let (tx, rx) = rc::alloc(channel);
    (Sender(tx), Receiver(rx))
}

struct Channel<T> {
    queue: queue::Queue<T>,
    receiver: WaitCell,
}

pub struct Sender<T>(rc::Sender<Channel<T>, { queue::MAX_SENDERS }>);

unsafe impl<T: Send> Send for Sender<T> {}
unsafe impl<T: Send> Sync for Sender<T> {}

impl<T> Sender<T> {
    pub fn send(&self, value: T) -> Result<(), SendError<T>> {
        if self.0.is_disconnected() {
            return Err(SendError(value));
        }

        self.0.queue.push(value);
        self.0.receiver.wake();
        Ok(())
    }
}

pub struct Receiver<T>(rc::Receiver<Channel<T>, 1>);

unsafe impl<T: Send> Send for Receiver<T> {}
// impl<T> !Sync for Receiver<T> {}

impl<T> Receiver<T> {
    pub fn try_recv(&self) -> Result<T, TryRecvError> {
        match unsafe { self.0.queue.pop() } {
            Some(value) => Ok(value),
            None if self.0.is_disconnected() => Err(TryRecvError::Disconnected),
            None => Err(TryRecvError::Empty),
        }
    }

    pub async fn recv(&self) -> Result<T, RecvError> {
        struct RecvFuture<'a, T>(&'a Receiver<T>);

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

    pub fn is_empty(&self) -> bool {
        unsafe { self.0.queue.is_empty() }
    }
}

impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        Sender(self.0.clone())
    }
}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        unsafe {
            self.0.drop(|| {
                self.0.receiver.wake();
            })
        }
    }
}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        unsafe { self.0.drop(|| {}) }
    }
}
