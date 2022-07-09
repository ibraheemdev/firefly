#![doc = include_str!("../README.md")]
#![allow(unstable_name_collisions, clippy::collapsible_if, dead_code, unused)]

pub mod mpfc;
pub mod mpsc;
pub mod spsc;

mod blocking;
mod error;
mod rc;
mod util;
mod wait;

pub use error::{RecvError, RecvTimeoutError, TryRecvError};
pub use error::{SendError, SendTimeoutError, TrySendError};
