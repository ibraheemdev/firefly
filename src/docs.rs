macro_rules! docs {
    ([$path:literal, $idents:literal] bounded) => {
concat!(
"Creates a channel of bounded capacity.

This channel has a buffer that can hold at most `capacity` messages at a time.

**Note**: The `capacity` must be a power of two, and cannot be zero.

# Examples

```",
"
use firefly::{", $path, "::bounded, TrySendError};",
"

// create a channel that can hold at most 2 messages at a time
let (", $idents, ") = bounded(2);

// the first two messages send immediately
tx.try_send(1).unwrap();
tx.try_send(2).unwrap();

// the third would exceed the capacity
assert_eq!(tx.try_send(3), Err(TrySendError::Full(3)));

// you can also wait until capacity is freed up, i.e. a message is received
// tx.send(3).await;
```
"
)
    };

    ([$path:literal, $idents:literal] unbounded) => {
concat!(
"Creates a channel of unbounded capacity.

This channel has a growable buffer that can hold any number of messages at a time.

# Examples

```",
"
use firefly::{", $path, "::unbounded, TrySendError};",
"

# #[tokio::main] async fn main() {
// create an unbounded channel
let (", $idents, ") = unbounded();

// send an arbitrary number of messages
for i in 0..1000 {
    tx.send(i).unwrap();
}

// disconnect the sender
drop(tx);

// receive all sent messages
while let Ok(i) = rx.recv().await {
    println!(\"{i}\",);
}
# }
```
"
)
    };

    ([$path:literal, $create:literal, $send:literal, $async:literal, $idents:literal] try_send) => {
concat!(
"Attempts to send a message into the channel without blocking.

This method will either send a message into the channel immediately or return an error
if the channel is full or disconnected. The returned error contains the original message.

# Examples

```
use firefly::{", $path, ", TrySendError};",
"

let (", $idents, ") = bounded(2);

// sending within the capacity is fine
assert_eq!(tx.try_send(1), Ok(()));
assert_eq!(tx.try_send(2), Ok(()));

// but not if it's full
assert_eq!(tx.try_send(3), Err(TrySendError::Full(3)));

// or the receiver is disconnected
drop(rx);
assert_eq!(tx.try_send(3), Err(TrySendError::Disconnected(3)));"
)
    };

    ([$path:literal, "unbounded()", $send:literal, $async:literal, $idents:literal] send) => {
concat!(
"Sends a message on this channel without blocking.

This method never blocks because the channel is unbounded and can never be full. If the receiver
is disconnected, an error will be returned containing the original message.

# Examples

```
# use tokio::{task, time::sleep};
use firefly::{", $path, ", SendError};",
"
# #[tokio::main] async fn main() {

let (", $idents, ") = unbounded();

// receive some number of messages, then disconnect
task::spawn(async move {
    for _ in 0..1000 {
        let _ = rx.recv().await;
    }
    drop(rx);
});

// send an arbitrary amount of messages
for i in 0.. {
    if let Err(x) = tx.send(i) {
        // until the receiver disconnts
        assert_eq!(x, SendError(i));
        println!(\"receiver disconnected\");
        break;
    }
}
# }
```"
)
    };

    ([$path:literal, "bounded(2)", $send:literal, $async:literal, $idents:literal] send) => {
concat!(
"Blocks the current task until a message is sent.

If the channel is full, this call will asynchronously wait until capacity frees up.
If the receiver disconnects, this call will wake up and return an error.
The returned error contains the original message.

# Examples

```
# use std::time::Duration;
# use tokio::{task, time::sleep};
use firefly::{", $path, ", SendError};",
"
# #[tokio::main] async fn main() {

let (", $idents, ") = bounded(2);

// fill up the channel
tx.try_send(1).unwrap();
tx.try_send(2).unwrap();

// receive a single message
task::spawn(async move {
    assert_eq!(rx.recv().await, Ok(1));
    sleep(Duration::from_secs(1)).await;
    drop(rx);
});

// sends once the channel has capacity, i.e. the message is received
assert_eq!(tx.send(3).await, Ok(()));

// tries to wait, but errors after 1 second when the receiver disconnects
assert_eq!(tx.send(4).await, Err(SendError(4)));
# }
```"
)
    };

    ([$path:literal, $create:literal, $send:literal, $async:literal, $idents:literal] send_blocking) => {
concat!(
"Blocks the current thread until a message is sent.

If the channel is full, this call will block until capacity frees up. If the receiver
disconnects, this call will wake up and return an error. The returned error
contains the original message.

This method is *blocking* and **should only** be used in a synchronous context. In an
asynchronous context, the [`send`] method should be used instead.

[`send`]: Sender::send

# Examples

```
# use std::time::Duration;
# use std::thread;
use firefly::{", $path, ", SendError};",
"

let (", $idents, ") = bounded(2);

// fill up the channel
tx.try_send(1).unwrap();
tx.try_send(2).unwrap();

// receive a single message
thread::spawn(move || {
    assert_eq!(rx.recv_blocking(), Ok(1));
    thread::sleep(Duration::from_secs(1));
    drop(rx);
});

// sends once the channel has capacity, i.e. the message is received
assert_eq!(tx.send_blocking(3), Ok(()));

// tries to wait, but errors after 1 second when the receiver disconnects
assert_eq!(tx.send_blocking(4), Err(SendError(4)));
```"
)
    };

    ([$path:literal, $create:literal, $send:literal, $async:literal, $idents:literal] send_blocking_timeout) => {
concat!(
"Blocks the current thread until a message is sent or the timeout expires.

If the channel is full, this call will asynchronously wait until capacity frees up, or the
timeout expires. If the receiver disconnects, this call will wake up and return an error.
The returned error contains the original message.

This method is *blocking* and **should only** be used in a synchronous context. In an
asynchronous context, the [`send`] method along with the `timeout` function provided by your
async runtime should be used instead.

[`send`]: Sender::send

# Examples

```
# use std::thread;
# use std::time::Duration;",
"
use firefly::{", $path, ", SendTimeoutError};",
"

let (", $idents, ") = bounded(2);

// fill up the channel
tx.try_send(1).unwrap();
tx.try_send(2).unwrap();

// receives a message after 1 second
thread::spawn(move || {
    thread::sleep(Duration::from_secs(1));
    assert_eq!(rx.recv_blocking(), Ok(2));
    drop(rx);
});

// a timeout of 500ms is not enough
assert_eq!(
    tx.send_blocking_timeout(3, Duration::from_millis(500)),
    Err(SendTimeoutError::Timeout(3)),
);

// but we can send after 1 second
assert_eq!(
    tx.send_blocking_timeout(4, Duration::from_secs(1)),
    Ok(()),
);

// now the channel is empty and the receiver disconnected
assert_eq!(
    tx.send_blocking_timeout(5, Duration::from_millis(500)),
    Err(SendTimeoutError::Disconnected(5)),
);
```"
)
    };

    ([$path:literal, $create:literal, $send:literal, $async:literal, $idents:literal] try_recv) => {
concat!(
"Attempts to receive a message from the channel without blocking.

This method will either receive a message from the channel immediately or return an error
if the channel is empty, or the sender is disconnected.

# Examples

```
use firefly::{", $path, ", TryRecvError};",
"

let (", $idents, ") = ", $create, ";

// the channel is empty
assert_eq!(rx.try_recv(), Err(TryRecvError::Empty));

// send a message and disconnect
tx.", $send, "(1).unwrap();
drop(tx);

// receive a message
assert_eq!(rx.try_recv(), Ok(1));

// the channel is empty and disconnected
assert_eq!(rx.try_recv(), Err(TryRecvError::Disconnected));
```"
)
    };

    ([$path:literal, $create:literal, $send:literal, $async:literal, $idents:literal] recv) => {
concat!(
"Blocks the current task until a message is received.

If the channel is empty, this call will asynchronously wait until a message is sent. If the sender
disconnects, this call will wake up and return an error.

# Examples

```
# use tokio::{task, time::sleep};
# use std::time::Duration;
use firefly::{", $path, ", RecvError};",
"

# #[tokio::main] async fn main() {
let (", $idents, ") = ", $create, ";

// send a message after 1 second, and disconnect
task::spawn(async move {
    sleep(Duration::from_secs(1)).await;
    tx.", $send, "(1).unwrap();
    drop(tx);
});

// wait until the message is sent
assert_eq!(rx.recv().await, Ok(1));

// the channel is now disconnected
assert_eq!(rx.recv().await, Err(RecvError))
# }
```"
)
    };

    ([$path:literal, $create:literal, $send:literal, $async:literal, $idents:literal] recv_blocking) => {
concat!(
"Blocks the current thread until a message is received.

If the channel is empty, this call will block until a message is sent. If the sender
disconnects, this call will wake up and return an error.

This method is *blocking* and **should only** be used in a synchronous context. In an
asynchronous context, [`recv`] should be used instead.

[`recv`]: ", $async,
"

# Examples

```
# use std::{thread, time::Duration};
use firefly::{", $path, ", RecvError};",
"

let (", $idents, ") = ", $create, ";

// send a message after 1 second, and disconnect
thread::spawn(move || {
    thread::sleep(Duration::from_secs(1));
    tx.", $send, "(1).unwrap();
    drop(tx);
});

// wait until the message is sent
assert_eq!(rx.recv_blocking(), Ok(1));

// the channel is now disconnected
assert_eq!(rx.recv_blocking(), Err(RecvError))
```"
)
    };

    ([$path:literal, $create:literal, $send:literal, $async:literal, $idents:literal] recv_blocking_timeout) => {
concat!(
"Blocks the current thread until a message is received or the timeout expires.

If the channel is empty, this call will block until a message is sent. If the sender
disconnects or the timeout expires, this call will wake up and return an error.

This method is *blocking* and **should only** be used in a synchronous context. In an
asynchronous context, [`recv`] should be used instead.

[`recv`]: ", $async,
"

# Examples

```
# use std::{thread, time::Duration};
use firefly::{", $path, ", RecvTimeoutError};",
"

let (", $idents, ") = ", $create, ";

// send a message after 1 second, then disconnect
thread::spawn(move || {
    thread::sleep(Duration::from_secs(1));
    tx.", $send, "(1).unwrap();
    drop(tx);
});

// a timeout of 500ms is not enough
assert_eq!(
    rx.recv_blocking_timeout(Duration::from_millis(500)),
    Err(RecvTimeoutError::Timeout),
);

// but we can receive after 1 second
assert_eq!(
    rx.recv_blocking_timeout(Duration::from_secs(1)),
    Ok(1),
);

// now the channel is empty and the sender is disconnected
assert_eq!(
    rx.recv_blocking_timeout(Duration::from_secs(1)),
    Err(RecvTimeoutError::Disconnected),
);
```"
)
    };

    (spsc::$func:ident) => {
        docs!(["spsc", "mut tx, mut rx"] $func)
    };

    (spsc::bounded::$func:ident) => {
        docs!(["spsc::bounded", "bounded(2)", "try_send", "Receiver::recv", "mut tx, mut rx"] $func)
    };

    (spsc::unbounded::$func:ident) => {
        docs!(["spsc::unbounded", "unbounded()", "send", "UnboundedReceiver::recv", "mut tx, mut rx"] $func)
    };

    (mpsc::$func:ident) => {
        docs!(["mpsc", "tx, mut rx"] $func)
    };

    (mpsc::bounded::$func:ident) => {
        docs!(["mpsc::bounded", "bounded(2)", "try_send", "Receiver::recv", "tx, mut rx"] $func)
    };

    (mpsc::unbounded::$func:ident) => {
        docs!(["mpsc::unbounded", "unbounded()", "send", "UnboundedReceiver::recv", "tx, mut rx"] $func)
    };

    (spmc::$func:ident) => {
        docs!(["spmc", "tx, rx"] $func)
    };

    (spmc::bounded::$func:ident) => {
        docs!(["spmc::bounded", "bounded(2)", "try_send", "Receiver::recv", "mut tx, rx"] $func)
    };

    (spmc::unbounded::$func:ident) => {
        docs!(["spmc::unbounded", "unbounded()", "send", "UnboundedReceiver::recv", "mut tx, rx"] $func)
    };

    (mpmc::$func:ident) => {
        docs!(["mpmc", "tx, rx"] $func)
    };

    (mpmc::bounded::$func:ident) => {
        docs!(["mpmc::bounded", "bounded(2)", "try_send", "Receiver::recv", "tx, rx"] $func)
    };

    (mpmc::unbounded::$func:ident) => {
        docs!(["mpmc::unbounded", "unbounded()", "send", "UnboundedReceiver::recv", "tx, rx"] $func)
    };

}
pub(crate) use docs;
