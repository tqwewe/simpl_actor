use std::{
    any::{self, Any},
    convert::Infallible,
    time::Duration,
};

use async_trait::async_trait;
use simpl_actor::{actor, Actor, ActorError, ActorRef, ActorStopReason, Spawn};

pub struct CounterActor {
    count: i64,
}

#[async_trait]
impl Actor for CounterActor {
    type Error = Infallible;

    async fn pre_start(&mut self) -> Result<(), Self::Error> {
        println!("starting actor {}", any::type_name::<Self>());
        Ok(())
    }

    async fn pre_restart(&mut self, _err: Box<dyn Any + Send>) -> Result<bool, Self::Error> {
        println!("restarting actor {}", any::type_name::<Self>());
        Ok(true)
    }

    async fn pre_stop(self, reason: ActorStopReason<Self::Error>) -> Result<(), Self::Error> {
        println!(
            "stopping actor {} because {reason}",
            any::type_name::<Self>()
        );
        Ok(())
    }
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

    #[message]
    pub async fn sleep(&self) {
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let counter = CounterActor::new();
    let actor = counter.spawn();
    assert_eq!(actor.inc(2).await, Ok(()));
    assert_eq!(actor.inc(5).await, Ok(()));
    assert_eq!(actor.dec(3).await, Ok(()));
    assert_eq!(actor.count().await, Ok(4));

    assert_eq!(actor.sleep_async().await, Ok(()));
    for _ in 0..CounterActor::channel_size() {
        assert_eq!(actor.inc_async(1).await, Ok(()));
    }

    assert_eq!(actor.try_inc(1).await, Err(ActorError::MailboxFull(1)));
    assert_eq!(
        actor.inc_timeout(1, Duration::from_millis(200)).await,
        Err(ActorError::Timeout(1))
    );

    actor.stop_immediately();
    actor.wait_for_stop().await;
    assert_eq!(actor.inc(1).await, Err(ActorError::ActorNotRunning(1)));
}
