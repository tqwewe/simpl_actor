use simpl_actor::{actor, Actor, Spawn};

#[derive(Actor)]
pub struct CounterActor {
    count: i64,
}

#[actor]
impl CounterActor {
    pub fn new() -> Self {
        CounterActor { count: 0 }
    }

    #[message]
    pub fn inc(&mut self, amount: i64) {
        self.count += amount;
    }

    #[message]
    pub fn dec(&mut self, amount: i64) {
        self.count -= amount;
    }

    #[message]
    pub fn count(&self) -> i64 {
        self.count
    }
}

#[tokio::main]
async fn main() {
    let actor = CounterActor::new().spawn();
    let _ = actor.inc(2).await;
    let _ = actor.inc(5).await;
    let _ = actor.dec(3).await;
    assert_eq!(actor.count().await, Ok(4));
}
