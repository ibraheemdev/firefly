mod shared;
use shared::*;

use std::sync::{Arc, Barrier};
use std::thread;

use criterion::measurement::WallTime;
use criterion::{criterion_group, criterion_main, BenchmarkGroup, Criterion};
use dry::macro_for;

fn run(c: &mut Criterion) {
    // // ========== MPSC/UNBOUNDED ==========
    // let mut group = c.benchmark_group("mpsc/unbounded");
    // group.sample_size(20);
    // macro_for!($X in [FireflyMpscUnbounded, FlumeUnbounded, StdMpscUnbounded, CrossbeamUnbounded] {
    //     mpsc::<$X>(&mut group, None);
    // });
    // group.finish();

    // // ========== MPSC/BOUNDED/UNCONTENDED ==========
    // let mut group = c.benchmark_group("mpsc/bounded/uncontended");
    // group.sample_size(20);
    // macro_for!($X in [FireflyMpsc, Flume, StdMpsc, Crossbeam, ThingbufMpsc] {
    //     mpsc::<$X>(&mut group, None);
    // });
    // group.finish();

    // // ========== MPSC/BOUNDED/CONTENDED ==========
    // let mut group = c.benchmark_group("mpsc/bounded/contended");
    // group.sample_size(20);
    // macro_for!($X in [FireflyMpsc, Flume, StdMpsc, Crossbeam, ThingbufMpsc] {
    //     mpsc::<$X>(&mut group, Some(10));
    // });
    // group.finish();

    // ========== SPSC/UNBOUNDED ==========
    let mut group = c.benchmark_group("spsc/unbounded");
    group.sample_size(20);
    macro_for!($X in [FireflySpsc, Flume, StdMpsc, Crossbeam, ThingbufMpsc] {
        spsc::<$X>(&mut group, Some(10));
    });
    group.finish();

    // ========== SPSC/BOUNDED/UNCONTENDED ==========
    let mut group = c.benchmark_group("spsc/bounded/uncontended");
    group.sample_size(20);
    macro_for!($X in [FireflySpsc, Flume, StdMpsc, Crossbeam, ThingbufMpsc] {
        spsc::<$X>(&mut group, None);
    });
    group.finish();

    // ========== SPSC/BOUNDED/CONTENDED ==========
    let mut group = c.benchmark_group("spsc/bounded/contended");
    group.sample_size(20);
    macro_for!($X in [FireflySpsc, Flume, StdMpsc, Crossbeam, ThingbufMpsc] {
        spsc::<$X>(&mut group, Some(10));
    });
    group.finish();
}

fn mpsc<C>(g: &mut BenchmarkGroup<'_, WallTime>, load: Option<usize>)
where
    C: Chan<usize>,
    C::Tx: Clone,
{
    let threads = 1.max(thread::available_parallelism().unwrap().get() - 2);
    let messages = threads * 50_000;
    let capacity = load
        .map(|load| messages / load)
        .unwrap_or(messages)
        .next_power_of_two();

    g.bench_function(C::NAME, |b| {
        b.iter(|| {
            let (tx, mut rx) = C::new(Some(capacity));
            let barrier = Arc::new(Barrier::new(threads + 1));

            thread::scope(|scope| {
                for _ in 0..threads {
                    let mut tx = tx.clone();
                    let barrier = barrier.clone();
                    scope.spawn(move || {
                        barrier.wait();
                        for i in 0..messages / threads {
                            tx._send(i).unwrap();
                        }
                    });
                }

                barrier.wait();
                for _ in 0..messages {
                    rx._recv().unwrap();
                }
            });
        })
    });
}

fn spsc<C>(g: &mut BenchmarkGroup<'_, WallTime>, load: Option<usize>)
where
    C: Chan<usize>,
{
    let messages = 500_000;
    let capacity = load
        .map(|load| messages / load)
        .unwrap_or(messages)
        .next_power_of_two();

    g.bench_function(C::NAME, |b| {
        b.iter(|| {
            let (mut tx, mut rx) = C::new(Some(capacity));
            let barrier = Arc::new(Barrier::new(2));

            thread::scope(|scope| {
                scope.spawn({
                    let barrier = barrier.clone();
                    move || {
                        barrier.wait();
                        for i in 0..messages {
                            tx._send(i).unwrap();
                        }
                    }
                });

                barrier.wait();
                for _ in 0..messages {
                    rx._recv().unwrap();
                }
            });
        })
    });
}

criterion_group!(benches, run);
criterion_main!(benches);
