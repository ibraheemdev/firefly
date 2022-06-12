mod queue;

use crate::error::*;
use crate::wait::WaitQueue;
use crate::{blocking, rc};

use std::task::Poll;

pub(super) fn new<T>(capacity: usize) -> (Sender<T>, Receiver<T>) {
    let channel = Channel {
        queue: queue::Queue::new(capacity),
        receivers: WaitQueue::new(),
        senders: WaitQueue::new(),
    };

    let (tx, rx) = rc::alloc(channel);
    (Sender(tx), Receiver(rx))
}

struct Channel<T> {
    queue: queue::Queue<T>,
    receivers: WaitQueue,
    senders: WaitQueue,
}

pub struct Sender<T>(rc::Sender<Channel<T>>);

unsafe impl<T: Send> Send for Sender<T> {}
unsafe impl<T: Send> Sync for Sender<T> {}

impl<T> Sender<T> {
    pub fn try_send(&self, value: T) -> Result<(), TrySendError<T>> {
        match self.0.queue.push(value) {
            Ok(_) => {
                self.0.receivers.wake();
                Ok(())
            }
            Err(value) if self.0.is_disconnected() => Err(TrySendError::Disconnected(value)),
            Err(value) => Err(TrySendError::Full(value)),
        }
    }

    pub async fn send(&self, value: T) -> Result<(), SendError<T>> {
        let mut state = Some(value);

        self.0
            .senders
            .poll_fn(|| {
                let value = state.take().unwrap();
                match self.try_send(value) {
                    Ok(()) => Poll::Ready(Ok(())),
                    Err(TrySendError::Disconnected(value)) => Poll::Ready(Err(SendError(value))),
                    Err(TrySendError::Full(value)) => {
                        state = Some(value);
                        Poll::Pending
                    }
                }
            })
            .await
    }

    pub fn send_blocking(&self, value: T) -> Result<(), SendError<T>> {
        unsafe { blocking::block_on(self.send(value)) }
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
                self.0.senders.wake();
                Ok(value)
            }
            None if self.0.is_disconnected() => Err(TryRecvError::Disconnected),
            None => Err(TryRecvError::Empty),
        }
    }

    pub async fn recv(&self) -> Result<T, RecvError> {
        self.0
            .receivers
            .poll_fn(|| match self.try_recv() {
                Ok(value) => Poll::Ready(Ok(value)),
                Err(TryRecvError::Disconnected) => Poll::Ready(Err(RecvError)),
                Err(TryRecvError::Empty) => Poll::Pending,
            })
            .await
    }

    pub fn recv_blocking(&self) -> Result<T, RecvError> {
        unsafe { blocking::block_on(self.recv()) }
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
        unsafe { self.0.drop(|| self.0.receivers.wake_all()) }
    }
}

impl<T> Clone for Receiver<T> {
    fn clone(&self) -> Self {
        Receiver(self.0.clone())
    }
}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        unsafe { self.0.drop(|| self.0.senders.wake_all()) }
    }
}
