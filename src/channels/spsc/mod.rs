mod bounded;
mod unbounded;

use crate::error::*;
use crate::raw::parking::task::{self, Task};
use crate::raw::{blocking, rc};

use std::task::Poll;
use std::time::Duration;

pub fn bounded<T>(capacity: usize) -> (Sender<T>, Receiver<T>) {
    let (tx, rx) = rc::alloc(Channel {
        queue: bounded::Queue::new(capacity),
        receiver: Task::new(),
        sender: Task::new(),
    });

    let sender = Sender {
        chan: tx,
        handle: bounded::Handle::new(),
    };

    let receiver = Receiver {
        chan: rx,
        handle: bounded::Handle::new(),
    };

    (sender, receiver)
}

struct Channel<T> {
    queue: bounded::Queue<T>,
    receiver: Task,
    sender: Task,
}

pub struct Sender<T> {
    chan: rc::Sender<Channel<T>, 1>,
    handle: bounded::Handle,
}

unsafe impl<T: Send> Send for Sender<T> {}

impl<T> Sender<T> {
    pub fn try_send(&self, value: T) -> Result<(), TrySendError<T>> {
        if self.chan.is_disconnected() {
            return Err(TrySendError::Disconnected(value));
        }

        unsafe { self.handle.push(value, &self.chan.queue) }
            .map(|_| self.chan.receiver.unpark())
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
        task::block_on!(self.chan.sender => || {
            let value = state.take().unwrap();
            match self.try_send(value) {
                Ok(()) => Poll::Ready(Ok(())),
                Err(TrySendError::Disconnected(value)) => Poll::Ready(Err(SendError(value))),
                Err(TrySendError::Full(value)) => {
                    *state = Some(value);
                    Poll::Pending
                }
            }
        })
    }
}

pub struct Receiver<T> {
    chan: rc::Receiver<Channel<T>, 1>,
    handle: bounded::Handle,
}

unsafe impl<T: Send> Send for Receiver<T> {}

impl<T> Receiver<T> {
    pub fn try_recv(&self) -> Result<T, TryRecvError> {
        match unsafe { self.handle.pop(&self.chan.queue) } {
            Some(value) => {
                self.chan.sender.unpark();
                Ok(value)
            }
            None if self.chan.is_disconnected() => {
                match unsafe { self.handle.pop(&self.chan.queue) } {
                    Some(value) => {
                        self.chan.sender.unpark();
                        Ok(value)
                    }
                    _ => Err(TryRecvError::Disconnected),
                }
            }
            None => Err(TryRecvError::Empty),
        }
    }

    pub async fn recv(&self) -> Result<T, RecvError> {
        task::block_on!(self.chan.receiver => || match self.try_recv() {
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
        self.chan.queue.is_empty()
    }
}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        unsafe { self.chan.drop(|| self.chan.receiver.unpark()) }
    }
}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        unsafe { self.chan.drop(|| self.chan.sender.unpark()) }
    }
}

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

pub struct UnboundedSender<T>(rc::Sender<UnboundedChannel<T>, 1>);

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

pub struct UnboundedReceiver<T>(rc::Receiver<UnboundedChannel<T>, 1>);

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
