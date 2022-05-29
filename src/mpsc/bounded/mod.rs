mod queue;

use std::sync::Arc;

pub(super) fn new<T>(capacity: usize) -> (Sender<T>, Receiver<T>) {
    todo!()
}

pub struct Sender<T> {
    queue: Arc<queue::Queue<T>>,
}
pub struct Receiver<T> {
    queue: Arc<queue::Queue<T>>,
}
