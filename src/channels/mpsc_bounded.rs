use crate::error::*;
use crate::raw::parking::queue::{self, TaskQueue};
use crate::raw::parking::task::{self, Task};
use crate::raw::queues::mpsc_bounded::Queue;
use crate::raw::{blocking, rc};

use std::task::Poll;
use std::time::Duration;

pub(super) fn new<T>(capacity: usize) -> (Sender<T>, Receiver<T>) {
    let (tx, rx) = rc::alloc(Channel {
        queue: Queue::new(capacity),
        receiver: Task::new(),
        senders: TaskQueue::new(),
    });

    (Sender(tx), Receiver(rx))
}

struct Channel<T> {
    queue: Queue<T>,
    receiver: Task,
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
            .map(|_| self.0.receiver.unpark())
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
            should_park: || !self.0.queue.can_push()
        })
    }
}

pub struct Receiver<T>(rc::Receiver<Channel<T>, 1>);

unsafe impl<T: Send> Send for Receiver<T> {}

impl<T> Receiver<T> {
    pub fn try_recv(&self) -> Result<T, TryRecvError> {
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
        unsafe { self.0.drop(|| self.0.receiver.unpark()) }
    }
}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        unsafe { self.0.drop(|| self.0.senders.unpark_all()) }
    }
}
