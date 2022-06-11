use std::{
    marker::PhantomPinned,
    ops::{Deref, DerefMut},
    pin::Pin,
    ptr::NonNull,
};

pub struct LinkedList<T> {
    head: Option<NonNull<Node<T>>>,
    tail: Option<NonNull<Node<T>>>,
}

pub struct Node<T> {
    prev: Option<NonNull<Node<T>>>,
    next: Option<NonNull<Node<T>>>,
    pub data: T,
    // https://github.com/rust-lang/rust/pull/82834
    _pin: PhantomPinned,
}

impl<T> Node<T> {
    pub fn new(data: T) -> Node<T> {
        Node {
            data,
            next: None,
            prev: None,
            _pin: PhantomPinned,
        }
    }
}

impl<T> LinkedList<T> {
    pub fn new() -> Self {
        LinkedList {
            head: None,
            tail: None,
        }
    }

    pub unsafe fn add_front(&mut self, node: Pin<&mut Node<T>>) {
        let node = node.get_unchecked_mut();
        node.next = self.head;
        node.prev = None;

        match self.head {
            Some(mut head) => head.as_mut().prev = Some(node.into()),
            None => {}
        };

        self.head = Some(node.into());

        if self.tail.is_none() {
            self.tail = Some(node.into());
        }
    }

    pub fn is_empty(&self) -> bool {
        if !self.head.is_none() {
            return false;
        }

        debug_assert!(self.tail.is_none());
        true
    }

    pub fn pop(&mut self) -> Option<&mut Node<T>> {
        // SAFETY: When the node was inserted it was promised that it is alive
        // until it gets removed from the list
        unsafe {
            let mut tail = self.tail?;
            self.tail = tail.as_mut().prev;

            let last_ref = tail.as_mut();
            match last_ref.prev {
                None => {
                    // This was the last node in the list
                    debug_assert_eq!(Some(last_ref.into()), self.head);
                    self.head = None;
                }
                Some(mut prev) => {
                    prev.as_mut().next = None;
                }
            }

            last_ref.prev = None;
            last_ref.next = None;
            Some(&mut *(last_ref as *mut Node<T>))
        }
    }

    pub unsafe fn remove(&mut self, node: Pin<&mut Node<T>>) -> bool {
        let node = node.get_unchecked_mut();
        match node.prev {
            None => {
                // This might be the first node in the list. If it is not, the
                // node is not in the list at all. Since our precondition is that
                // the node must either be in this list or in no list, we check that
                // the node is really in no list.
                if self.head != Some(node.into()) {
                    debug_assert!(node.next.is_none());
                    return false;
                }
                self.head = node.next;
            }
            Some(mut prev) => {
                debug_assert_eq!(prev.as_ref().next, Some(node.into()));
                prev.as_mut().next = node.next;
            }
        }

        match node.next {
            None => {
                // This must be the last node in our list. Otherwise the list
                // is inconsistent.
                debug_assert_eq!(self.tail, Some(node.into()));
                self.tail = node.prev;
            }
            Some(mut next) => {
                debug_assert_eq!(next.as_mut().prev, Some(node.into()));
                next.as_mut().prev = node.prev;
            }
        }

        node.next = None;
        node.prev = None;

        true
    }

    pub fn drain<F>(&mut self, mut func: F)
    where
        F: FnMut(&mut Node<T>),
    {
        let mut current = self.tail;
        self.head = None;
        self.tail = None;

        while let Some(mut node) = current {
            // SAFETY: The nodes have not been removed from the list yet and must
            // therefore contain valid data. The nodes can also not be added to
            // the list again during iteration, since the list is mutably borrowed.
            unsafe {
                let node_ref = node.as_mut();
                current = node_ref.prev;

                node_ref.next = None;
                node_ref.prev = None;

                // Note: We do not reset the pointers from the next element in the
                // list to the current one since we will iterate over the whole
                // list anyway, and therefore clean up all pointers.

                func(node_ref);
            }
        }
    }
}
