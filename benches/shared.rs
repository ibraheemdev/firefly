pub trait Sender<T>: Send {
    fn send_blocking(&self, value: T) -> Result<(), ()>;
}

pub trait Receiver<T> {
    fn recv_blocking(&self) -> Result<T, ()>;
}

macro_rules! impl_chan {
    ($tx:ty |$x:ident, $val:ident| $send:block; $( $rx:ty |$y:ident| $recv:block; )?) => {
        impl<T: Clone + Default + Send + Sync> Sender<T> for $tx {
            fn send_blocking(&self, $val: T) -> Result<(), ()> {
                let $x = self;
                $send
            }
        }

        $(impl<T: Clone + Default> Receiver<T> for $rx {
            fn recv_blocking(&self) -> Result<T, ()> {
                let $y = self;
                $recv
            }
        })?
    };
    ($tx:ty) => {
        impl_chan! { $tx |c, val| { c.send(val).map_err(drop) }; }
    };
    ($tx:ty, $rx:ty) => {
        impl_chan! {
            $tx |c, val| { c.send(val).map_err(drop) };
            $rx |c| { c.recv().map_err(drop) };
        }
    };
}

impl_chan! { std::sync::mpsc::SyncSender<T> }
impl_chan! { std::sync::mpsc::Sender<T>, std::sync::mpsc::Receiver<T> }
impl_chan! { crossbeam::channel::Sender<T>, crossbeam::channel::Receiver<T> }
impl_chan! { flume::Sender<T>, flume::Receiver<T> }
impl_chan! {
    firefly::spsc::UnboundedSender<T> |c, val| { c.send(val).map_err(drop) };
    firefly::spsc::UnboundedReceiver<T> |c| { c.recv_blocking().map_err(drop) };
}
impl_chan! {
    firefly::spsc::Sender<T> |c, val| { c.send_blocking(val).map_err(drop) };
    firefly::spsc::Receiver<T> |c| { c.recv_blocking().map_err(|e| panic!("{}", e)) };
}
impl_chan! {
    firefly::mpsc::UnboundedSender<T> |c, val| { c.send(val).map_err(drop) };
    firefly::mpsc::UnboundedReceiver<T> |c| { c.recv_blocking().map_err(drop) };
}
impl_chan! {
    firefly::mpsc::Sender<T> |c, val| { c.send_blocking(val).map_err(drop) };
    firefly::mpsc::Receiver<T> |c| { c.recv_blocking().map_err(drop) };
}
impl_chan! {
    firefly::mpfc::UnboundedSender<T> |c, val| { c.send(val).map_err(drop) };
    firefly::mpfc::UnboundedReceiver<T> |c| { c.recv_blocking().map_err(drop) };
}
impl_chan! {
    firefly::mpfc::Sender<T> |c, val| { c.send_blocking(val).map_err(drop) };
    firefly::mpfc::Receiver<T> |c| { c.recv_blocking().map_err(drop) };
}
impl_chan! {
    thingbuf::mpsc::blocking::Sender<T> |c, val| { c.send(val).map_err(drop) };
    thingbuf::mpsc::blocking::Receiver<T> |c| { c.recv().ok_or(()) };
}
