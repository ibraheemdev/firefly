use firefly::{mpmc, mpsc, spsc};

use static_assertions::{assert_impl_all, assert_not_impl_any};

struct NotSend(*mut ());
unsafe impl Sync for NotSend {}

#[test]
fn auto_traits() {
    // spsc
    assert_impl_all!(spsc::Sender<()>: Send);
    assert_not_impl_any!(spsc::Sender<()>: Sync, Clone);
    assert_not_impl_any!(spsc::Sender<NotSend>: Send);

    assert_impl_all!(spsc::Receiver<()>: Send);
    assert_not_impl_any!(spsc::Receiver<()>: Sync, Clone);
    assert_not_impl_any!(spsc::Receiver<NotSend>: Send);

    assert_impl_all!(spsc::UnboundedSender<()>: Send);
    assert_not_impl_any!(spsc::UnboundedSender<()>: Sync, Clone);
    assert_not_impl_any!(spsc::UnboundedSender<NotSend>: Send);

    assert_impl_all!(spsc::UnboundedReceiver<()>: Send);
    assert_not_impl_any!(spsc::UnboundedReceiver<()>: Sync, Clone);
    assert_not_impl_any!(spsc::UnboundedReceiver<NotSend>: Send);

    // mpsc
    assert_impl_all!(mpsc::Sender<()>: Send, Sync, Clone);
    assert_not_impl_any!(mpsc::Sender<NotSend>: Send, Sync);

    assert_impl_all!(mpsc::UnboundedSender<()>: Send, Sync, Clone);
    assert_not_impl_any!(mpsc::UnboundedSender<NotSend>: Send, Sync);

    assert_impl_all!(mpsc::Receiver<()>: Send);
    assert_not_impl_any!(mpsc::Receiver<()>: Sync, Clone);
    assert_not_impl_any!(mpsc::Receiver<NotSend>: Send, Sync, Clone);

    assert_impl_all!(mpsc::UnboundedReceiver<()>: Send);
    assert_not_impl_any!(mpsc::UnboundedReceiver<()>: Sync, Clone);
    assert_not_impl_any!(mpsc::UnboundedReceiver<NotSend>: Send, Sync, Clone);

    // mpmc
    assert_impl_all!(mpmc::Sender<()>: Send, Sync, Clone);
    assert_not_impl_any!(mpmc::Sender<NotSend>: Send, Sync);

    assert_impl_all!(mpmc::UnboundedSender<()>: Send, Sync, Clone);
    assert_not_impl_any!(mpmc::UnboundedSender<NotSend>: Send, Sync);

    assert_impl_all!(mpmc::Receiver<()>: Send, Sync, Clone);
    assert_not_impl_any!(mpmc::Receiver<NotSend>: Send, Sync);

    assert_impl_all!(mpmc::UnboundedReceiver<()>: Send, Sync, Clone);
    assert_not_impl_any!(mpmc::UnboundedReceiver<NotSend>: Send, Sync);
}
