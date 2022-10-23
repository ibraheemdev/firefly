use crossbeam::channel as crossbeam;
use std::sync::mpsc;

pub trait Chan<T> {
    const NAME: &'static str;

    type Tx: Sender<T>;
    type Rx: Receiver<T>;

    fn new(cap: Option<usize>) -> (Self::Tx, Self::Rx);
}

pub trait Sender<T>: Send {
    fn _send(&mut self, value: T) -> Result<(), ()>;
}

pub trait Receiver<T> {
    fn _recv(&mut self) -> Result<T, ()>;
}

// std::sync::mpsc
impl_sender! { mpsc::Sender<T> => send }
impl_sender! { mpsc::SyncSender<T> => send }
impl_receiver! { mpsc::Receiver<T> => recv }
impl_chan! { StdMpsc(mpsc::SyncSender<T>, mpsc::Receiver<T>) => |cap| mpsc::sync_channel(cap.unwrap()) }
impl_chan! { StdMpscUnbounded(mpsc::Sender<T>, mpsc::Receiver<T>) => |_cap| mpsc::channel() }

// crossbeam::channel
impl_sender! { crossbeam::Sender<T> => send }
impl_receiver! { crossbeam::Receiver<T> => recv }
impl_chan! { Crossbeam(crossbeam::Sender<T>, crossbeam::Receiver<T>) => |cap| crossbeam::bounded(cap.unwrap()) }
impl_chan! { CrossbeamUnbounded(crossbeam::Sender<T>, crossbeam::Receiver<T>) => |_cap| crossbeam::unbounded() }

// flume
impl_sender! { flume::Sender<T> => send }
impl_receiver! { flume::Receiver<T> => recv }
impl_chan! { Flume(flume::Sender<T>, flume::Receiver<T>) => |cap| flume::bounded(cap.unwrap()) }
impl_chan! { FlumeUnbounded(flume::Sender<T>, flume::Receiver<T>) => |_cap| flume::unbounded() }

// firefly::spsc
impl_sender! { firefly::spsc::Sender<T> => send_blocking, firefly::spsc::UnboundedSender<T> => send }
impl_receiver! { firefly::spsc::Receiver<T> => recv_blocking, firefly::spsc::UnboundedReceiver<T> => recv_blocking }
impl_chan! { FireflySpsc(firefly::spsc::Sender<T>, firefly::spsc::Receiver<T>) => |cap| firefly::spsc::bounded(cap.unwrap()) }
impl_chan! { FireflySpscUnbounded(firefly::spsc::UnboundedSender<T>, firefly::spsc::UnboundedReceiver<T>) => |_cap| firefly::spsc::unbounded() }

// firefly::mpsc
impl_sender! { firefly::mpsc::Sender<T> => send_blocking, firefly::mpsc::UnboundedSender<T> => send }
impl_receiver! { firefly::mpsc::Receiver<T> => recv_blocking, firefly::mpsc::UnboundedReceiver<T> => recv_blocking }
impl_chan! { FireflyMpsc(firefly::mpsc::Sender<T>, firefly::mpsc::Receiver<T>) => |cap| firefly::mpsc::bounded(cap.unwrap()) }
impl_chan! { FireflyMpscUnbounded(firefly::mpsc::UnboundedSender<T>, firefly::mpsc::UnboundedReceiver<T>) => |_cap| firefly::mpsc::unbounded() }

// firefly::mpmc
impl_sender! { firefly::mpmc::Sender<T> => send_blocking, firefly::mpmc::UnboundedSender<T> => send }
impl_receiver! { firefly::mpmc::Receiver<T> => recv_blocking, firefly::mpmc::UnboundedReceiver<T> => recv_blocking }
impl_chan! { FireflyMpmc(firefly::mpmc::Sender<T>, firefly::mpmc::Receiver<T>) => |cap| firefly::mpmc::bounded(cap.unwrap()) }
impl_chan! { FireflyMpmcUnbounded(firefly::mpmc::UnboundedSender<T>, firefly::mpmc::UnboundedReceiver<T>) => |_cap| firefly::mpmc::unbounded() }

// thingbuf::mpsc
impl_sender! { thingbuf::mpsc::blocking::Sender<T> => send }
impl_receiver! { thingbuf::mpsc::blocking::Receiver<T> => |x| x.recv().ok_or(()) }
impl_chan! { ThingbufMpsc(thingbuf::mpsc::blocking::Sender<T>, thingbuf::mpsc::blocking::Receiver<T>) => |cap| thingbuf::mpsc::blocking::channel(cap.unwrap()) }

macro_rules! impl_chan {
    ($chan:ident($tx:ty, $rx:ty) => |$cap:ident| $new:expr) => {
        pub struct $chan;

        impl<T: Clone + Default + Send + Sync> Chan<T> for $chan {
            const NAME: &'static str = stringify!($chan);
            type Tx = $tx;
            type Rx = $rx;
            fn new($cap: Option<usize>) -> (Self::Tx, Self::Rx) {
                $new
            }
        }
    };
}

macro_rules! impl_receiver {
    ($($rx:ty => |$x:ident| $recv:expr),*) => {
        $(impl<T: Clone + Default> Receiver<T> for $rx {
            fn _recv(&mut self) -> Result<T, ()> {
                let $x = self;
                $recv
            }
        })*
    };
    ($($rx:ty => $recv:ident),*) => {
        $(impl_receiver! { $rx => |x| x.$recv().map_err(|e| panic!("{e:?}")) })*
    };
}

macro_rules! impl_sender {
    ($($tx:ty => |$x:ident, $val:ident| $send:expr),*) => {
        $(impl<T: Clone + Default + Send + Sync> Sender<T> for $tx {
            fn _send(&mut self, $val: T) -> Result<(), ()> {
                let $x = self;
                $send
            }
        })*
    };
    ($($tx:ty => $send:ident),*) => {
        $(impl_sender! { $tx => |x, val| x.$send(val).map_err(drop) })*
    };
}

use {impl_chan, impl_receiver, impl_sender};
