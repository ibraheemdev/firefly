mod cell;
mod queue;

pub use cell::WaitCell;
pub use queue::WaitQueue;

pub enum Status {
    Registered,
    Awoke,
}
