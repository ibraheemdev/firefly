pub trait Sender<T>: Clone + Send {
    fn send(&self, value: T) -> Result<(), ()>;
}

pub trait Receiver<T> {
    fn recv(&self) -> Result<T, ()>;
}

macro_rules! impl_chan {
    ($tx:ty |$x:ident, $val:ident| $send:block; $rx:ty |$y:ident| $recv:block;) => {
        impl<T: Clone + Default + Send + Sync> Sender<T> for $tx {
            fn send(&self, $val: T) -> Result<(), ()> {
                let $x = self;
                $send
            }
        }

        impl<T: Clone + Default> Receiver<T> for $rx {
            fn recv(&self) -> Result<T, ()> {
                let $y = self;
                $recv
            }
        }
    };
    ($tx:ty, $rx:ty) => {
        impl_chan! {
            $tx |c, val| { c.send(val).map_err(drop) };
            $rx |c| { c.recv().map_err(drop) };
        }
    };
}

impl_chan! { std::sync::mpsc::Sender<T>, std::sync::mpsc::Receiver<T> }
impl_chan! { crossbeam::channel::Sender<T>, crossbeam::channel::Receiver<T> }
impl_chan! { flume::Sender<T>, flume::Receiver<T> }
impl_chan! {
    firefly::mpsc::unbounded::Sender<T> |c, val| { c.send(val).map_err(drop) };
    firefly::mpsc::unbounded::Receiver<T> |c| { c.recv_blocking().map_err(drop) };
}
impl_chan! {
    thingbuf::mpsc::blocking::Sender<T> |c, val| { c.send(val).map_err(drop) };
    thingbuf::mpsc::blocking::Receiver<T> |c| { c.recv().ok_or(()) };
}
