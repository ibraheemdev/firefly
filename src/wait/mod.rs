pub mod mpmc;
pub mod mpsc;
pub mod spsc;

use std::task::Waker;

fn will_wake(curr: &Option<Waker>, new: &Waker) -> bool {
    curr.as_ref().map(|w| w.will_wake(new)).unwrap_or(false)
}
