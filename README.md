# `firefly`

A collection of high performance concurrent channels.

```rust
// create an unbounded channel
let (tx, rx) = firefly::mpsc::unbounded();

thread::spawn(move || {
    // send a message across
    tx.send(42).unwrap();
});

// receive the message
assert_eq!(rx.recv_blocking().unwrap(), 42);
```

## Channel Flavors

Firefly provides a variety of channel flavors, optimized for specific use cases:

- [`mpsc::bounded`]
- [`mpsc::unbounded`]
- [`mpmc::bounded`]
- [`mpmc::unbounded`]

In general a channel flavor higher up on the list is likely to be more performant
than a more generic one lower down.

## Bounded Channels

Bounded channels are created with a bounded capacity; the maximum number of messages
that can be held at a given time:

```rust
// create a channel that can hold at most 10 messages at a time
let (tx, rx) = firefly::mpsc::bounded(10);

thread::spawn(move || {
    for i in 0..100 {
        // send a message, potentially blocking until capacity frees up
        tx.send_blocking(i).unwrap();
    }
});

for _ in 0..100 {
    // wait for a message to be sent
    let i = rx.recv_blocking().unwrap();
    println!("{i}");
}
```

## Unbounded Channels

Unbounded channels on the other hand are unlimited in their capacity, meaning that
sending never blocks:

```rust
// create an unbounded channel
let (tx, rx) = firefly::mpsc::unbounded();

thread::spawn(move || {
    // send an infinite amount of messages
    for i in 0.. {
        tx.send(i).unwrap();
    }
});

// receive an infinite amount of messages
loop {
    let i = rx.recv_blocking().unwrap();
    println!("{i}");
}
```

## Blocking

Send and receive operations can be performed four different ways:

- Non-blocking (returns immediately with success or failure).
- Blocking (blocks the thread until the operation succeeds or the channel disconnects).
- Blocking with a timeout (blocks upto a maximum duration of time).
- Asynchronously (blocks the async task).

```rust
use std::time::Duration;

let (tx, rx) = firefly::mpsc::bounded(4);

thread::spawn(move || {
    for _ in 0..3 {
        // this can never fail because we only ever send
        // 3 messages, and the capacity is 4
        tx.try_send(42).unwrap();
    }
});

// receive the message or return an error if not immediately ready
match rx.try_recv() {
    Ok(x) => assert_eq!(x, 42),
    Err(_) => println!("message has not been sent yet")
}

// block the thread until the message is sent
assert_eq!(rx.recv_blocking().unwrap(), 42);

// block the thread upto 100ms
match rx.recv_timeout(Duration::from_millis(100)) {
    Ok(x) => assert_eq!(x, 42),
    Err(_) => println!("message took too long to send")
}

tokio::spawn(async move {
    // block the async task until the message is sent
    assert_eq!(rx.recv().await.unwrap(), 42);
});
```

Channels can also be used as "bridge" channels between async and sync code:

```rust
let (tx, rx) = firefly::mpsc::bounded(10);

thread::spawn(move || {
    // send messages synchronously
    for i in 0.. {
        tx.send_blocking(i).unwrap()
    }
});

tokio::spawn(async move {
    // receive asynchronously
    loop {
        let i = rx.recv().await.unwrap();
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
let (tx, rx) = firefly::mpsc::unbounded();
tx.send(1).unwrap();
tx.send(2).unwrap();

// drop the sender
drop(tx);

// any remaining messages can be received
assert_eq!(r.recv(), Ok(1));
assert_eq!(r.recv(), Ok(2));

// subsequent attempts will error
assert_eq!(r.recv(), Err(RecvError));
```
