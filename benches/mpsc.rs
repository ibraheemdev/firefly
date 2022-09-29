mod shared;
use shared::{Receiver, Sender};

use std::sync::{Arc, Barrier};

use criterion::{
    criterion_group, criterion_main, measurement::Measurement, BenchmarkGroup, Criterion,
};

fn bench_all(c: &mut Criterion) {
    let mut group = c.benchmark_group("mpsc/unbounded");
    group.sample_size(20);

    bench("firefly", &mut group, |_| firefly::mpsc::unbounded());
    bench("firefly-mpmc", &mut group, |_| firefly::mpmc::unbounded());
    bench("flume", &mut group, |_| flume::unbounded());
    bench("std::sync::mpsc", &mut group, |_| {
        std::sync::mpsc::channel()
    });
    bench("crossbeam-channel", &mut group, |_| {
        crossbeam::channel::unbounded()
    });

    group.finish();

    let mut group = c.benchmark_group("mpsc/bounded/uncontended");
    group.sample_size(20);

    bench("firefly", &mut group, |x| firefly::mpsc::bounded(x));
    bench("firefly-mpmc", &mut group, |x| firefly::mpmc::bounded(x));
    bench("flume", &mut group, |x| flume::bounded(x));
    bench("std::sync::mpsc", &mut group, |x| {
        std::sync::mpsc::sync_channel(x)
    });
    bench("thingbuf", &mut group, |x| {
        thingbuf::mpsc::blocking::channel(x)
    });
    bench("crossbeam-channel", &mut group, |x| {
        crossbeam::channel::bounded(x)
    });

    group.finish();

    const LOAD: usize = 10;
    let mut group = c.benchmark_group("mpsc/bounded/contended");
    group.sample_size(20);

    bench("firefly", &mut group, |x| firefly::mpsc::bounded(x / LOAD));
    bench("firefly-mpmc", &mut group, |x| {
        firefly::mpmc::bounded(x / LOAD)
    });
    bench("flume", &mut group, |x| flume::bounded(x / LOAD));
    bench("std::sync::mpsc", &mut group, |x| {
        std::sync::mpsc::sync_channel(x / LOAD)
    });
    bench("thingbuf", &mut group, |x| {
        thingbuf::mpsc::blocking::channel(x / LOAD)
    });
    bench("crossbeam-channel", &mut group, |x| {
        crossbeam::channel::bounded(x / LOAD)
    });

    group.finish();
}

fn bench<M, S, R>(name: &'static str, g: &mut BenchmarkGroup<'_, M>, chan: impl Fn(usize) -> (S, R))
where
    M: Measurement,
    S: Sender<usize> + Send + Clone,
    R: Receiver<usize>,
{
    let threads = (std::thread::available_parallelism().unwrap().get() - 2).max(1);
    let messages = threads * 50_000;

    g.bench_function(name, |b| {
        b.iter(|| {
            let (tx, rx) = chan(messages);
            let barrier = Arc::new(Barrier::new(threads + 1));

            crossbeam::scope(|scope| {
                for _ in 0..threads {
                    let tx = tx.clone();
                    let barrier = barrier.clone();
                    scope.spawn(move |_| {
                        barrier.wait();
                        for i in 0..messages / threads {
                            tx.send_blocking(i).unwrap();
                        }
                    });
                }

                barrier.wait();
                for _ in 0..messages {
                    rx.recv_blocking().unwrap();
                }
            })
            .unwrap();
        })
    });
}

criterion_group!(benches, bench_all);
criterion_main!(benches);
