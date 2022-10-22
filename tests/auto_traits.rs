use firefly::{mpmc, mpsc, spmc, spsc};

use static_assertions::{assert_impl_all, assert_not_impl_any};

struct SendNotSync(*mut ());
unsafe impl Sync for SendNotSync {}

#[test]
fn auto_traits() {
    // spsc
    assert_impl_all!(spsc::Sender<()>: Send, Sync);
    assert_not_impl_any!(spsc::Sender<()>: Clone);
    assert_not_impl_any!(spsc::Sender<SendNotSync>: Send);

    assert_impl_all!(spsc::Receiver<()>: Send, Sync);
    assert_not_impl_any!(spsc::Receiver<()>: Clone);
    assert_not_impl_any!(spsc::Receiver<SendNotSync>: Send);

    assert_impl_all!(spsc::UnboundedSender<()>: Send, Sync);
    assert_not_impl_any!(spsc::UnboundedSender<()>: Clone);
    assert_not_impl_any!(spsc::UnboundedSender<SendNotSync>: Send);

    assert_impl_all!(spsc::UnboundedReceiver<()>: Send, Sync);
    assert_not_impl_any!(spsc::UnboundedReceiver<()>: Clone);
    assert_not_impl_any!(spsc::UnboundedReceiver<SendNotSync>: Send);

    // mpsc
    assert_impl_all!(mpsc::Sender<()>: Send, Sync, Clone);
    assert_not_impl_any!(mpsc::Sender<SendNotSync>: Send, Sync);

    assert_impl_all!(mpsc::UnboundedSender<()>: Send, Sync, Clone);
    assert_not_impl_any!(mpsc::UnboundedSender<SendNotSync>: Send, Sync);

    assert_impl_all!(mpsc::Receiver<()>: Send, Sync);
    assert_not_impl_any!(mpsc::Receiver<()>: Clone);
    assert_not_impl_any!(mpsc::Receiver<SendNotSync>: Send, Sync, Clone);

    assert_impl_all!(mpsc::UnboundedReceiver<()>: Send, Sync);
    assert_not_impl_any!(mpsc::UnboundedReceiver<()>: Clone);
    assert_not_impl_any!(mpsc::UnboundedReceiver<SendNotSync>: Send, Sync, Clone);

    // spmc
    assert_impl_all!(spmc::Sender<()>: Send, Sync);
    assert_not_impl_any!(spmc::Sender<()>: Clone);
    assert_not_impl_any!(spmc::Sender<SendNotSync>: Send, Sync);

    assert_impl_all!(spmc::UnboundedSender<()>: Send, Sync);
    assert_not_impl_any!(spmc::UnboundedSender<()>: Clone);
    assert_not_impl_any!(spmc::UnboundedSender<SendNotSync>: Send, Sync);

    assert_impl_all!(spmc::Receiver<()>: Send, Sync, Clone);
    assert_not_impl_any!(spmc::Receiver<SendNotSync>: Send, Sync);

    assert_impl_all!(spmc::UnboundedReceiver<()>: Send, Sync, Clone);
    assert_not_impl_any!(spmc::UnboundedReceiver<SendNotSync>: Send, Sync);

    // mpmc
    assert_impl_all!(mpmc::Sender<()>: Send, Sync, Clone);
    assert_not_impl_any!(mpmc::Sender<SendNotSync>: Send, Sync);

    assert_impl_all!(mpmc::UnboundedSender<()>: Send, Sync, Clone);
    assert_not_impl_any!(mpmc::UnboundedSender<SendNotSync>: Send, Sync);

    assert_impl_all!(mpmc::Receiver<()>: Send, Sync, Clone);
    assert_not_impl_any!(mpmc::Receiver<SendNotSync>: Send, Sync);

    assert_impl_all!(mpmc::UnboundedReceiver<()>: Send, Sync, Clone);
    assert_not_impl_any!(mpmc::UnboundedReceiver<SendNotSync>: Send, Sync);
}
