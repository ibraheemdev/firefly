//! Multi-producer, single-consumer channels.

mod bounded;
mod unbounded;

pub use bounded::{Receiver, Sender};
pub use unbounded::{UnboundedReceiver, UnboundedSender};

pub fn unbounded<T>() -> (UnboundedSender<T>, UnboundedReceiver<T>) {
    unbounded::new()
}

pub fn bounded<T>(capacity: usize) -> (Sender<T>, Receiver<T>) {
    bounded::new(capacity)
}
