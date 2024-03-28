use criterion::black_box;
use criterion::BenchmarkId;
use criterion::Criterion;
use criterion::{criterion_group, criterion_main};
use simpl_actor::*;

#[derive(Actor)]
struct FibActor {}

#[actor]
impl FibActor {
    #[message(infallible)]
    fn fib(&mut self, n: u64) -> u64 {
        fibonacci(n)
    }
}

fn fibonacci(n: u64) -> u64 {
    match n {
        0 => 1,
        1 => 1,
        n => fibonacci(n - 1) + fibonacci(n - 2),
    }
}

fn from_elem(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _guard = rt.enter();
    let size: usize = 1024;
    let actor_ref = FibActor {}.spawn();
    c.bench_with_input(BenchmarkId::new("input_example", size), &size, |b, _| {
        // Insert a call to `to_async` to convert the bencher to async mode.
        // The timing loops are the same as with the normal bencher.
        b.to_async(&rt).iter(|| async {
            tokio::try_join!(
                actor_ref.fib(black_box(20)),
                actor_ref.fib(black_box(20)),
                actor_ref.fib(black_box(20)),
                actor_ref.fib(black_box(20)),
                actor_ref.fib(black_box(20)),
                actor_ref.fib(black_box(20)),
                actor_ref.fib(black_box(20)),
                actor_ref.fib(black_box(20)),
                actor_ref.fib(black_box(20)),
                actor_ref.fib(black_box(20)),
                actor_ref.fib(black_box(20)),
                actor_ref.fib(black_box(20)),
                actor_ref.fib(black_box(20)),
                actor_ref.fib(black_box(20)),
                actor_ref.fib(black_box(20)),
                actor_ref.fib(black_box(20)),
                actor_ref.fib(black_box(20)),
                actor_ref.fib(black_box(20)),
                actor_ref.fib(black_box(20)),
                actor_ref.fib(black_box(20)),
                actor_ref.fib(black_box(20)),
                actor_ref.fib(black_box(20)),
                actor_ref.fib(black_box(20)),
                actor_ref.fib(black_box(20)),
                actor_ref.fib(black_box(20)),
                actor_ref.fib(black_box(20)),
                actor_ref.fib(black_box(20)),
                actor_ref.fib(black_box(20)),
                actor_ref.fib(black_box(20)),
                actor_ref.fib(black_box(20)),
                actor_ref.fib(black_box(20)),
                actor_ref.fib(black_box(20)),
            )
            .unwrap();
        });
    });
}

criterion_group!(benches, from_elem);
criterion_main!(benches);
