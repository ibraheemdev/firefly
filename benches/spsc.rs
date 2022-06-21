mod shared;
use shared::{Receiver, Sender};

use std::sync::{Arc, Barrier};

use criterion::{
    criterion_group, criterion_main, measurement::Measurement, BenchmarkGroup, Criterion,
};

fn bench_all(c: &mut Criterion) {
    let mut group = c.benchmark_group("spsc/unbounded");
    group.sample_size(20);

    bench("firefly", &mut group, |_| firefly::spsc::unbounded());
    bench("firefly-mpsc", &mut group, |_| firefly::mpsc::unbounded());

    group.finish();

    let mut group = c.benchmark_group("spsc/bounded/uncontended");
    group.sample_size(20);

    bench("firefly", &mut group, |x| firefly::spsc::bounded(x));
    bench("firefly-mpsc", &mut group, |x| firefly::mpsc::bounded(x));

    group.finish();

    let mut group = c.benchmark_group("spsc/bounded/contended");
    group.sample_size(20);

    bench("firefly", &mut group, |x| firefly::spsc::bounded(x / 2));
    bench("firefly-mpsc", &mut group, |x| {
        firefly::mpsc::bounded(x / 2)
    });

    group.finish();
}

fn bench<M, S, R>(name: &'static str, g: &mut BenchmarkGroup<'_, M>, chan: impl Fn(usize) -> (S, R))
where
    M: Measurement,
    S: Sender<usize>,
    R: Receiver<usize>,
{
    let messages = 500_000;

    g.bench_function(name, |b| {
        b.iter(|| {
            let (tx, rx) = chan(messages);
            let barrier = Arc::new(Barrier::new(2));

            crossbeam::scope(|scope| {
                scope.spawn({
                    let barrier = barrier.clone();
                    move |_| {
                        barrier.wait();
                        for i in 0..messages {
                            tx.send(i).unwrap();
                        }
                    }
                });

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
