use std::{
    any::{self, Any},
    convert::Infallible,
    time::Duration,
};

use async_trait::async_trait;
use simpl_actor::{actor, Actor, ActorError, ActorRef, ActorStopReason, ShouldRestart, Spawn};

pub struct CounterActor {
    count: i64,
}

#[async_trait]
impl Actor for CounterActor {
    type Error = Infallible;

    async fn on_start(&mut self) -> Result<(), Self::Error> {
        println!("starting actor {}", any::type_name::<Self>());
        Ok(())
    }

    async fn on_panic(&mut self, _err: Box<dyn Any + Send>) -> Result<ShouldRestart, Self::Error> {
        println!("restarting actor {}", any::type_name::<Self>());
        Ok(ShouldRestart::Yes)
    }

    async fn on_stop(self, reason: ActorStopReason<Self::Error>) -> Result<(), Self::Error> {
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

    /// A message with no return value
    #[message]
    pub fn inc(&mut self, amount: i64) {
        self.count += amount;
    }

    /// A message returning a value
    #[message]
    pub fn count(&self) -> i64 {
        self.count
    }

    /// A message that uses async
    #[message]
    pub async fn sleep(&self) {
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    /// A message that sends a message to itself
    #[message]
    pub async fn inc_myself(&self) -> Result<(), ActorError<i64>> {
        self.actor_ref().inc_async(1).await
    }

    /// A message that panics
    #[message]
    pub fn force_panic(&self) {
        panic!("forced panic, don't worry this is correct and the actor will be restarted")
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let counter = CounterActor::new();
    let actor = counter.spawn();

    // Increment
    assert_eq!(actor.inc(2).await, Ok(()));
    // Count should be 2
    assert_eq!(actor.count().await, Ok(2));

    // Trigger the actor to sleep for 500ms in the background
    assert_eq!(actor.sleep_async().await, Ok(()));
    // Fill the mailbox with messages
    for _ in 0..CounterActor::mailbox_size() {
        assert_eq!(actor.inc_async(1).await, Ok(()));
    }
    // Mailbox should be full, so if we try to send a message without backpressure using the try_ method,
    // we should get an error that the mailbox is full
    assert_eq!(actor.try_inc(1).await, Err(ActorError::MailboxFull(1)));
    // And if we try to increment, waiting for 200ms for there to be capacity, we should timeout since the actor was sleeping for 500ms
    assert_eq!(
        actor.inc_timeout(1, Duration::from_millis(200)).await,
        Err(ActorError::Timeout(1))
    );

    // If a panic occurs when running a message, we should receive an error that the actor was sopped
    assert_eq!(actor.force_panic().await, Err(ActorError::ActorStopped));
    // But we've implemented the `Actor::pre_restart` method to return `true`, so the actor will be restarted,
    // and new messages should be handled sucessfully, even with the state being preserved
    assert_eq!(actor.inc(1).await, Ok(()));
    assert_eq!(actor.count().await, Ok(67));

    // Stop the actor, dropping any pending messages
    actor.stop_immediately();
    // Await the actor to stop
    actor.wait_for_stop().await;
    // Any new messages should error since the actor is no longer running
    assert_eq!(actor.inc(1).await, Err(ActorError::ActorNotRunning(1)));
}
