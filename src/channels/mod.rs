pub mod mpmc;
pub mod mpsc;
pub mod spsc;

// /// Multi-producer multi-consumer channels.
// pub mod mpmc {
//     pub use super::mpmc_bounded::{Receiver, Sender};
//     pub use super::mpmc_unbounded::{UnboundedReceiver, UnboundedSender};
//
//     /// Create an unbounded multi-producer multi-consumer queue.
//     pub fn unbounded<T>() -> (UnboundedSender<T>, UnboundedReceiver<T>) {
//         super::mpmc_unbounded::new()
//     }
//
//     /// Create a multi-producer multi-consumer queue with a fixed capacity.
//     pub fn bounded<T>(capacity: usize) -> (Sender<T>, Receiver<T>) {
//         super::mpmc_bounded::new(capacity)
//     }
// }
//
// /// Multi-producer single-consumer channels.
// pub mod mpsc {
//     pub use super::mpsc_bounded::{Receiver, Sender};
//     pub use super::mpsc_unbounded::{UnboundedReceiver, UnboundedSender};
//
//     /// Create an unbounded multi-producer single-consumer queue.
//     pub fn unbounded<T>() -> (UnboundedSender<T>, UnboundedReceiver<T>) {
//         super::mpsc_unbounded::new()
//     }
//
//     /// Create a multi-producer single-consumer queue with a fixed capacity.
//     pub fn bounded<T>(capacity: usize) -> (Sender<T>, Receiver<T>) {
//         super::mpsc_bounded::new(capacity)
//     }
// }
//
// /// Single-producer single-consumer channels.
// pub mod spsc {
//     pub use super::spsc_bounded::{Receiver, Sender};
//     pub use super::spsc_unbounded::{UnboundedReceiver, UnboundedSender};
//
//     /// Create an unbounded single-producer single-consumer queue.
//     pub fn unbounded<T>() -> (UnboundedSender<T>, UnboundedReceiver<T>) {
//         super::spsc_unbounded::new()
//     }
//
//     /// Create a single-producer single-consumer queue with a fixed capacity.
//     pub fn bounded<T>(capacity: usize) -> (Sender<T>, Receiver<T>) {
//         super::spsc_bounded::new(capacity)
//     }
// }
