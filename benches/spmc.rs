mod shared;
use shared::{Receiver, Sender};

use std::sync::{Arc, Barrier};

use criterion::{
    criterion_group, criterion_main, measurement::Measurement, BenchmarkGroup, Criterion,
};

fn bench_all(c: &mut Criterion) {
    let mut group = c.benchmark_group("spmc/bounded/uncontended");
    group.sample_size(20);

    bench("firefly", &mut group, |x| firefly::spmc::bounded(x));
    bench("firefly-mpmc", &mut group, |x| firefly::mpmc::bounded(x));
    // bench("flume", &mut group, |x| flume::bounded(x));
    // bench("std::sync::mpsc", &mut group, |x| {
    //     std::sync::mpsc::sync_channel(x)
    // });
    bench("crossbeam-channel", &mut group, |x| {
        crossbeam::channel::bounded(x)
    });

    group.finish();

    let mut group = c.benchmark_group("spmc/bounded/contended");
    group.sample_size(20);

    bench("firefly", &mut group, |x| firefly::spmc::bounded(x / 2));
    bench("firefly-mpmc", &mut group, |x| {
        firefly::mpmc::bounded(x / 2)
    });
    // bench("flume", &mut group, |x| flume::bounded(x / 2));
    // bench("std::sync::mpsc", &mut group, |x| {
    //     std::sync::mpsc::sync_channel(x / 2)
    // });
    bench("crossbeam-channel", &mut group, |x| {
        crossbeam::channel::bounded(x / 2)
    });

    group.finish();
}

fn bench<M, S, R>(name: &'static str, g: &mut BenchmarkGroup<'_, M>, chan: impl Fn(usize) -> (S, R))
where
    M: Measurement,
    S: Sender<usize>,
    R: Receiver<usize> + Send + Clone,
{
    let threads = (std::thread::available_parallelism().unwrap().get() - 2).max(1);
    let messages = threads * 50_000;

    g.bench_function(name, |b| {
        b.iter(|| {
            let (tx, rx) = chan(messages);
            let barrier = Arc::new(Barrier::new(threads + 1));

            crossbeam::scope(|scope| {
                for _ in 0..threads {
                    let rx = rx.clone();
                    let barrier = barrier.clone();
                    scope.spawn(move |_| {
                        barrier.wait();
                        for _ in 0..messages / threads {
                            rx.recv().unwrap();
                        }
                    });
                }

                barrier.wait();
                for i in 0..messages {
                    tx.send(i).unwrap();
                }
            })
            .unwrap();
        })
    });
}

criterion_group!(benches, bench_all);
criterion_main!(benches);
