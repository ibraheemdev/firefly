#![allow(unstable_name_collisions, clippy::collapsible_if)]

pub mod mpmc;
pub mod mpsc;

mod blocking;
mod error;
mod rc;
mod util;
mod wait;

pub use error::{RecvError, RecvTimeoutError, TryRecvError};
pub use error::{SendError, SendTimeoutError, TrySendError};
