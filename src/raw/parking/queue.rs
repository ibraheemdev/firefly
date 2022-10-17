use super::signal::Signal;
use crate::raw::blocking;
use crate::raw::intrusive::{List, Node};
use crate::raw::util::{UnsafeDeref, UnsafeSend};

use std::cell::{Cell, UnsafeCell};
use std::future::Future;
use std::mem::drop;
use std::pin::Pin;
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::task::{Context, Poll, Waker};

use usync::Mutex;

/// A FIFO queue of parked tasks.
pub struct TaskQueue {
    // the number of pending tasks in/entering the queue
    pending: Signal<AtomicUsize>,
    // the queue of tasks
    tasks: Mutex<List<Task>>,
}

unsafe impl Send for TaskQueue {}
unsafe impl Sync for TaskQueue {}

/// A parked task.
struct Task {
    // A flag indicating whether or not the task is in the queue.
    parked: Cell<bool>,
    state: AtomicU8,
    waker: UnsafeCell<Option<Waker>>,
}

/// Asynchronously 'block' until a resource is ready, parking the
/// task if it is not.
///
// This is a macro for better inlining behavior (which seems hit and miss with async fns)
macro_rules! block_on {
    ($queue:expr => { poll: || $poll:expr, should_park: || $should_park:expr }) => {{
        loop {
            match { $poll } {
                Poll::Ready(value) => return value,
                Poll::Pending => {
                    std::hint::spin_loop();
                    $queue.park(|| $should_park).await;
                }
            }
        }
    }};
}

pub(crate) use block_on;

const EMPTY: u8 = 0;
const WAITING: u8 = 1;
const UPDATING: u8 = 2;
const NOTIFIED: u8 = 3;

impl TaskQueue {
    /// Create a empty queue.
    pub fn new() -> Self {
        Self {
            pending: Signal::new(0),
            tasks: Mutex::new(List::new()),
        }
    }

    /// Block the current task until it is unparked by another thread.
    pub fn park<'a>(
        &'a self,
        should_park: impl FnOnce() -> bool + 'a,
    ) -> impl Future<Output = ()> + 'a {
        UnsafeSend(async move {
            self.pending.inner().fetch_add(1, Ordering::Acquire);
            let mut tasks = self.tasks.lock();

            if !should_park() {
                drop(tasks);
                self.pending.inner().fetch_sub(1, Ordering::Relaxed);
                return;
            }

            unsafe {
                let mut task = Node::new(Task {
                    parked: Cell::new(true),
                    state: AtomicU8::new(EMPTY),
                    waker: Default::default(),
                });

                let mut task = Pin::new_unchecked(&mut task);
                tasks.push(task.as_mut());

                drop(tasks);

                Park {
                    parker: self,
                    task: Some(task),
                }
                .await;
            }
        })
    }

    /// Remove and unpark a single task from the queue.
    pub fn unpark_one(&self) {
        // if there are no pending tasks, the next task will acquire
        // our updates when it increments the count and retry.
        if self.pending.load_release() == 0 {
            return;
        }

        let mut tasks = self.tasks.lock();
        if let Some(task) = tasks.pop() {
            assert!(task.data.parked.replace(false));
            let task = &task.data as *const Task;
            drop(tasks);
            self.pending.inner().fetch_sub(1, Ordering::Relaxed);
            unsafe { task.deref().unpark() }
        }
    }

    /// Remove and unpark all tasks in the queue.
    pub fn unpark_all(&self) {
        if self.pending.load_release() == 0 {
            return;
        }

        let mut tasks = self.tasks.lock();
        let mut woke = 0;
        tasks.drain(|task| {
            assert!(task.data.parked.replace(false));
            task.data.unpark();
            woke += 1;
        });

        if woke > 0 {
            self.pending.inner().fetch_sub(woke, Ordering::Relaxed);
        }
    }
}

struct Park<'a, 'b> {
    parker: &'a TaskQueue,
    task: Option<Pin<&'b mut Node<Task>>>,
}

impl Future for Park<'_, '_> {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let task = self
            .task
            .take()
            .expect("ParkFuture polled after completion");

        if task.data.poll(cx.waker()).is_ready() {
            return Poll::Ready(());
        }

        self.task = Some(task);
        Poll::Pending
    }
}

impl Drop for Park<'_, '_> {
    fn drop(&mut self) {
        if let Some(mut task) = self.task.take() {
            unsafe {
                let mut lock = self.parker.tasks.lock();
                if task.as_ref().data.parked.replace(false) {
                    assert!(lock.remove(task.as_mut()));
                    self.parker.pending.inner().fetch_sub(1, Ordering::Relaxed);
                    return;
                }
            }

            self.task = Some(task);
            unsafe { blocking::block_on(self) }
        }
    }
}

// A simpler implementation of `super::Task` that can only be woken once.
impl Task {
    fn poll(&self, waker: &Waker) -> Poll<()> {
        let state = self.state.load(Ordering::Acquire);
        if state == NOTIFIED {
            return Poll::Ready(());
        }

        assert!(state == EMPTY || state == WAITING);
        if let Err(state) =
            self.state
                .compare_exchange(state, UPDATING, Ordering::Acquire, Ordering::Acquire)
        {
            assert_eq!(state, NOTIFIED);
            return Poll::Ready(());
        }

        unsafe {
            let curr = self.waker.get().deref_mut();
            if curr.as_ref().filter(|w| w.will_wake(waker)).is_none() {
                *curr = Some(waker.clone());
            }
        }

        match self
            .state
            .compare_exchange(UPDATING, WAITING, Ordering::AcqRel, Ordering::Acquire)
        {
            Ok(_) => Poll::Pending,
            Err(NOTIFIED) => Poll::Ready(()),
            Err(_) => unreachable!(),
        }
    }

    fn unpark(&self) {
        let state = self.state.swap(NOTIFIED, Ordering::AcqRel);
        if state != WAITING {
            return;
        }

        if let Some(waker) = unsafe { self.waker.get().deref_mut().take() } {
            waker.wake();
        }
    }
}
