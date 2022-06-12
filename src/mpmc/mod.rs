//! Multi-producer, multi-consumer channels.

pub mod bounded;

pub fn bounded<T>(capacity: usize) -> (bounded::Sender<T>, bounded::Receiver<T>) {
    bounded::new(capacity)
}
