#![allow(unstable_name_collisions)]
#![doc = include_str!("../README.md")]

mod channels;
mod error;
mod raw;

pub use channels::*;
pub use error::{RecvError, RecvTimeoutError, TryRecvError};
pub use error::{SendError, SendTimeoutError, TrySendError};
