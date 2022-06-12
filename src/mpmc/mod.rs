//! Multi-producer, multi-consumer channels.

pub mod bounded;
pub mod unbounded;

pub fn unbounded<T>() -> (unbounded::Sender<T>, unbounded::Receiver<T>) {
    unbounded::new()
}

pub fn bounded<T>(capacity: usize) -> (bounded::Sender<T>, bounded::Receiver<T>) {
    bounded::new(capacity)
}
