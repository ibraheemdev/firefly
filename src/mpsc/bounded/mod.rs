mod queue;

use crate::error::*;
use crate::wait::mpmc::WaitQueue;
use crate::wait::mpsc::WaitCell;
use crate::{blocking, rc, util};

use std::task::{Context, Poll};
use std::time::Duration;

pub(super) fn new<T>(capacity: usize) -> (Sender<T>, Receiver<T>) {
    let (tx, rx) = rc::alloc(Channel {
        queue: queue::Queue::new(capacity),
        receiver: WaitCell::new(),
        senders: WaitQueue::new(),
    });

    (Sender(tx), Receiver(rx))
}

struct Channel<T> {
    queue: queue::Queue<T>,
    receiver: WaitCell,
    senders: WaitQueue,
}

pub struct Sender<T>(rc::Sender<Channel<T>>);

unsafe impl<T: Send> Send for Sender<T> {}
unsafe impl<T: Send> Sync for Sender<T> {}

impl<T> Sender<T> {
    pub fn try_send(&self, value: T) -> Result<(), TrySendError<T>> {
        match self.0.queue.push(value) {
            Ok(_) => {
                self.0.receiver.wake();
                Ok(())
            }
            Err(value) if self.0.is_disconnected() => Err(TrySendError::Disconnected(value)),
            Err(value) => Err(TrySendError::Full(value)),
        }
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
        self.0
            .senders
            .poll_fn(
                || !self.0.queue.can_push(),
                || {
                    let value = state.take().unwrap();
                    match self.try_send(value) {
                        Ok(()) => Poll::Ready(Ok(())),
                        Err(TrySendError::Disconnected(value)) => {
                            Poll::Ready(Err(SendError(value)))
                        }
                        Err(TrySendError::Full(value)) => {
                            *state = Some(value);
                            Poll::Pending
                        }
                    }
                },
            )
            .await
    }
}

pub struct Receiver<T>(rc::Receiver<Channel<T>, 1>);

unsafe impl<T: Send> Send for Receiver<T> {}

impl<T> Receiver<T> {
    pub fn try_recv(&self) -> Result<T, TryRecvError> {
        match unsafe { self.0.queue.pop() } {
            Some(value) => {
                self.0.senders.wake();
                Ok(value)
            }
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
        unsafe { self.0.drop(|| self.0.receiver.wake()) }
    }
}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        unsafe { self.0.drop(|| self.0.senders.wake_all()) }
    }
}
