mod shared;
use shared::{Receiver, Sender};

use std::sync::{Arc, Barrier};

use criterion::{
    criterion_group, criterion_main, measurement::Measurement, BenchmarkGroup, Criterion,
};

fn bench_all(c: &mut Criterion) {
    let mut group = c.benchmark_group("mpsc/unbounded");
    group.sample_size(20);

    bench_unbounded("flume", &mut group, |_| flume::unbounded());
    bench_unbounded("firefly", &mut group, |_| firefly::mpsc::unbounded());
    bench_unbounded("std::sync::mpsc", &mut group, |_| {
        std::sync::mpsc::channel()
    });
    bench_unbounded("thingbuf", &mut group, |messages| {
        thingbuf::mpsc::blocking::channel(messages)
    });
    bench_unbounded("crossbeam-channel", &mut group, |_| {
        crossbeam::channel::unbounded()
    });
    group.finish();
}

fn bench_unbounded<M, S, R>(
    name: &'static str,
    g: &mut BenchmarkGroup<'_, M>,
    chan: impl Fn(usize) -> (S, R),
) where
    M: Measurement,
    S: Sender<usize>,
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
                            tx.send(i).unwrap();
                        }
                    });
                }

                barrier.wait();
                for _ in 0..messages {
                    rx.recv().unwrap();
                }
            })
            .unwrap();
        })
    });
}

criterion_group!(benches, bench_all);
criterion_main!(benches);
