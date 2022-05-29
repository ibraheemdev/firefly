#![allow(dead_code, unused, unstable_name_collisions)]

pub mod mpsc;

mod error;
mod managed;
mod utils;
mod waker;
mod blocking;

pub use error::{RecvError, RecvTimeoutError, TryRecvError};
pub use error::{SendError, SendTimeoutError, TrySendError};
