# `firefly`

A collection of high performance concurrent channels.

```rust
// create a SPSC channel with a capacity of 2
let (mut tx, mut rx) = firefly::spsc::bounded(2);

task::spawn(async move {
    // send a message across asynchronously
    tx.send(42).await.unwrap();
});

// receive the message synchronously
assert_eq!(rx.recv_blocking().unwrap(), 42);
```

## Channel Flavors

Firefly provides a variety of channel flavors, optimized for specific use cases:

- [`spsc::bounded`](https://docs.rs/firefly/latest/firefly/spsc/fn.bounded.html)
- [`spsc::unbounded`](https://docs.rs/firefly/latest/firefly/spsc/fn.unbounded.html)
- [`mpsc::bounded`](https://docs.rs/firefly/latest/firefly/mpsc/fn.bounded.html)
- [`mpsc::unbounded`](https://docs.rs/firefly/latest/firefly/mpsc/fn.unbounded.html)
- [`mpmc::bounded`](https://docs.rs/firefly/latest/firefly/mpmc/fn.bounded.html)
- [`mpmc::unbounded`](https://docs.rs/firefly/latest/firefly/mpmc/fn.unbounded.html)

In general, a channel flavor higher up on the list is likely to be more performant
than a more generic one lower down.

---

Bounded channels are created with a bounded capacity; the maximum number of messages
that can be held at a given time:

```rust
// create a channel that can hold at most 8 messages at a time
let (mut tx, mut rx) = firefly::spsc::bounded(8);

task::spawn(async move {
    for i in 0..100 {
        // send a message, potentially waiting until capacity frees up
        tx.send(i).await.unwrap();
    }
});

// block until messages are sent
while let Ok(i) = rx.recv_blocking() {
    println!("{i}");
}
```

Unbounded channels on the other hand are unlimited in their capacity, meaning that
sending never blocks:

```rust
// create an unbounded channel
let (mut tx, mut rx) = firefly::spsc::unbounded();

task::spawn(async move {
    // send an arbitrary amount of messages
    for i in 0..10_000 {
        tx.send(i).unwrap();
    }
});

// block until all messages are sent
while let Ok(i) = rx.recv_blocking() {
    println!("{i}");
}
```

## Blocking

Send and receive operations can be performed four different ways:

- Non-blocking (returns immediately with success or failure).
- Asynchronously (blocks the async task).
- Blocking (blocks the thread until the operation succeeds or the channel disconnects).
- Blocking with a timeout (blocks upto a maximum duration of time).

```rust
let (mut tx, mut rx) = firefly::spsc::bounded(4);

thread::spawn(move || {
    for _ in 0..3 {
        // this can never fail because we never exceed the capacity
        tx.try_send(42).unwrap();
    }
});

// attempt to receive the message without blocking
match rx.try_recv() {
    Ok(x) => assert_eq!(x, 42),
    Err(_) => println!("message has not been sent yet")
}

// block until the message is sent
assert_eq!(rx.recv_blocking(), Ok(42));

// block for at most 1 second
match rx.recv_blocking_timeout(Duration::from_secs(1)) {
    Ok(x) => assert_eq!(x, 42),
    Err(_) => println!("message took too long to send")
}

// spawn a task that receives the message asynchronously
task::spawn(async move {
    assert_eq!(rx.recv().await, Ok(42));
});
```

All channels can be used to "bridge" between async and sync code:

```rust
let (mut tx, mut rx) = firefly::spsc::bounded(8);

// send messages synchronously
thread::spawn(move || {
    for i in 0..16 {
        tx.send_blocking(i).unwrap()
    }
});

// receive asynchronously
task::spawn(async move {
    while let Ok(i) = rx.recv().await {
        println!("{i}");
    }
});
```

## Disconnection

When all senders or receivers of a given channel are dropped, the channel is
disconnected. Any attempts to send a message will fail. Any remaining messages
in the channel can be received, but subsequent attempts to receive will also
fail:

```rust
let (mut tx, mut rx) = firefly::spsc::unbounded();

tx.send(1).unwrap();
tx.send(2).unwrap();

// disconnect the sender
drop(tx);

// any remaining messages can be received
assert_eq!(rx.recv().await, Ok(1));
assert_eq!(rx.recv().await, Ok(2));

// subsequent attempts will error
assert_eq!(rx.recv().await, Err(firefly::RecvError));
```
