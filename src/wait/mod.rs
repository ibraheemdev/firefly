mod cell;
mod queue;

pub use cell::Cell;
pub use queue::{waiter, Queue, Waiter};

pub enum Status {
    Registered,
    Awoke,
}
