mod queue;

use crate::error::*;
use crate::{blocking, managed, wait};

use std::future::Future;
use std::hint;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

pub(super) fn new<T>(capacity: usize) -> (Sender<T>, Receiver<T>) {
    let channel = Channel {
        queue: queue::Queue::new(capacity),
        receiver: wait::Cell::new(),
        senders: wait::Queue::new(),
    };

    let (tx, rx) = managed::manage(channel);
    (Sender(tx), Receiver(rx))
}

struct Channel<T> {
    queue: queue::Queue<T>,
    receiver: wait::Cell,
    senders: wait::Queue,
}

pub struct Sender<T>(managed::Sender<Channel<T>, { usize::MAX }>);

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
        pin_project_lite::pin_project! {
            struct SendFuture<'a, T> {
                sender: &'a Sender<T>,
                state: State,
                value: Option<T>,
                #[pin]
                waiter: wait::Waiter,
            }

            impl<'a, T> PinnedDrop for SendFuture<'a, T> {
                fn drop(mut this: Pin<&mut Self>) {
                    let this = this.project();
                    if *this.state == State::Registered {
                        unsafe { this.sender.0.senders.remove(this.waiter) }
                    }
                }
            }
        }

        #[derive(PartialEq)]
        enum State {
            Started,
            Registered,
        }

        impl<'a, T> Future for SendFuture<'a, T> {
            type Output = Result<(), SendError<T>>;

            fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
                loop {
                    let this = self.as_mut().project();
                    let chan = &this.sender.0;

                    match this.state {
                        State::Started => {
                            let value = this.value.take().unwrap();
                            match this.sender.try_send(value) {
                                Ok(()) => return Poll::Ready(Ok(())),
                                Err(TrySendError::Disconnected(value)) => {
                                    return Poll::Ready(Err(SendError(value)))
                                }
                                Err(TrySendError::Full(value)) => {
                                    *this.value = Some(value);
                                }
                            }

                            match chan.senders.register(this.waiter, cx.waker()) {
                                wait::Status::Registered => {
                                    *this.state = State::Registered;
                                }
                                wait::Status::Awoke => {
                                    hint::spin_loop();
                                    continue;
                                }
                            }
                        }
                        State::Registered => {
                            if chan.senders.poll(this.waiter, cx.waker()).is_pending() {
                                return Poll::Pending;
                            }

                            let value = this.value.take().unwrap();
                            match this.sender.try_send(value) {
                                Ok(()) => return Poll::Ready(Ok(())),
                                Err(TrySendError::Disconnected(value)) => {
                                    return Poll::Ready(Err(SendError(value)))
                                }
                                Err(TrySendError::Full(value)) => {
                                    *this.value = Some(value);
                                    *this.state = State::Started;
                                }
                            }
                        }
                    }
                }
            }
        }

        SendFuture {
            sender: self,
            value: Some(value),
            waiter: wait::waiter(),
            state: State::Started,
        }
        .await
    }

    pub fn send_blocking(&self, value: T) -> Result<(), SendError<T>> {
        unsafe { blocking::block_on(self.send(value)) }
    }
}

pub struct Receiver<T>(managed::Receiver<Channel<T>, 1>);

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

            fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
                self.0.poll_recv(cx)
            }
        }

        RecvFuture(&self).await
    }

    pub fn poll_recv(&self, cx: &mut Context<'_>) -> Poll<Result<T, RecvError>> {
        loop {
            match unsafe { self.0.queue.pop() } {
                Some(value) => {
                    self.0.senders.wake();
                    return Poll::Ready(Ok(value));
                }
                _ if self.0.is_disconnected() => return Poll::Ready(Err(RecvError)),
                _ => {}
            }

            match unsafe { self.0.receiver.register(cx.waker()) } {
                wait::Status::Registered => return Poll::Pending,
                wait::Status::Awoke => hint::spin_loop(),
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
        unsafe {
            self.0.drop(|| {
                self.0.senders.wake_all();
            })
        }
    }
}
