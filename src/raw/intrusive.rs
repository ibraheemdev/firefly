// Copyright (c) 2019 Matthias Einwag
//
// Permission is hereby granted, free of charge, to any person
// obtaining a copy of this software and associated documentation
// files (the "Software"), to deal in the Software without restriction,
// including without limitation the rights to use, copy, modify, merge,
// publish, distribute, sublicense, and/or sell copies of the Software,
// and to permit persons to whom the Software is furnished to do so,
// subject to the following conditions:
//
// The above copyright notice and this permission notice shall be
// included in all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND,
// EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES
// OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT.
// IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR
// ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF
// CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION
// WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.

use std::marker::PhantomPinned;
use std::pin::Pin;
use std::ptr::NonNull;

/// An intrusive linked list.
pub struct List<T> {
    head: Option<NonNull<Node<T>>>,
    tail: Option<NonNull<Node<T>>>,
}

/// A list node.
pub struct Node<T> {
    pub data: T,
    prev: Option<NonNull<Node<T>>>,
    next: Option<NonNull<Node<T>>>,
    // https://github.com/rust-lang/rust/pull/82834
    _pin: PhantomPinned,
}

impl<T> Node<T> {
    /// Creates a new `Node` with the given data.
    pub fn new(data: T) -> Node<T> {
        Node {
            data,
            next: None,
            prev: None,
            _pin: PhantomPinned,
        }
    }
}

impl<T> List<T> {
    /// Creates a new list.
    pub fn new() -> Self {
        List {
            head: None,
            tail: None,
        }
    }

    /// Adds a node to the front of the list.
    ///
    /// # Safety
    ///
    /// The node must be removed from the list before it is moved, dropped,
    /// or added to another list.
    pub unsafe fn push(&mut self, node: Pin<&mut Node<T>>) {
        let node = node.get_unchecked_mut();
        node.next = self.head;
        node.prev = None;

        if let Some(mut head) = self.head {
            head.as_mut().prev = Some(node.into())
        };

        self.head = Some(node.into());

        if self.tail.is_none() {
            self.tail = Some(node.into());
        }
    }

    /// Removes the last from the list.
    pub fn pop(&mut self) -> Option<&mut Node<T>> {
        // safety: When the node was inserted it promised that it is alive
        // until it gets removed from the list
        unsafe {
            let mut tail = self.tail?.as_mut();
            self.tail = tail.prev;

            match tail.prev {
                Some(mut prev) => prev.as_mut().next = None,
                // this was the last node in the list
                None => {
                    debug_assert_eq!(self.head, Some(tail.into()));
                    self.head = None;
                }
            }

            tail.prev = None;
            tail.next = None;
            Some(tail)
        }
    }

    /// Removes the given node from the list, returning whether
    /// or not the node was removed.
    ///
    /// # Safety
    ///
    /// The node must either be part of this list, or no list at all.
    /// Passing a node from another list instance is undefined behavior.
    pub unsafe fn remove(&mut self, node: Pin<&mut Node<T>>) -> bool {
        let node = node.get_unchecked_mut();
        match node.prev {
            None => {
                // this should be the first node in the list
                if self.head != Some(node.into()) {
                    // if it is not, then it must not be part of any list
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
            Some(mut next) => {
                debug_assert_eq!(next.as_mut().prev, Some(node.into()));
                next.as_mut().prev = node.prev;
            }
            None => {
                // this must be the last node in our list, otherwise the list
                // is inconsistent
                debug_assert_eq!(self.tail, Some(node.into()));
                self.tail = node.prev;
            }
        }

        node.next = None;
        node.prev = None;

        true
    }

    /// Removes all nodes from this list, executing a callback for each node.
    pub fn drain<F>(&mut self, mut func: F)
    where
        F: FnMut(&mut Node<T>),
    {
        let mut current = self.tail;
        self.head = None;
        self.tail = None;

        while let Some(mut node) = current {
            // safety: the nodes have not been removed from the list yet and must
            // therefore contain valid data. the nodes can also not be added to
            // the list again during iteration, since the list is mutably borrowed
            unsafe {
                let node = node.as_mut();
                current = node.prev;

                node.next = None;
                node.prev = None;

                // note: we do not reset the pointers from the next element in the
                // list to the current one since we will iterate over the whole
                // list anyway, and therefore clean up all pointers
                func(node);
            }
        }
    }
}
