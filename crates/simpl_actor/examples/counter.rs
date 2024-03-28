//! This example shows a basic actor with different messages.

use std::time::Duration;

use simpl_actor::*;

pub struct CounterActor {
    count: i64,
}

impl Actor for CounterActor {
    type Ref = CounterActorRef;

    async fn on_start(&mut self) -> Result<(), BoxError> {
        println!("starting actor");
        Ok(())
    }

    async fn on_panic(&mut self, _err: PanicErr) -> Result<Option<ActorStopReason>, BoxError> {
        println!("actor panicked but will continue running");
        Ok(None)
    }

    async fn on_stop(self, reason: ActorStopReason) -> Result<(), BoxError> {
        println!("stopping actor because {reason}",);
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

    /// A message that takes borrowed inputs
    #[message]
    pub fn borrow_data<'a>(&self, s: &'a str) {
        println!("borrowed str: {s}");
    }

    /// A message returning an infallible value
    #[message(infallible)]
    pub fn count(&self) -> i64 {
        self.count
    }

    /// A message returning a Result
    #[message]
    pub fn error(&self) -> Result<(), std::io::Error> {
        Err(std::io::Error::new(std::io::ErrorKind::Other, "oh no!"))
    }

    /// A message that uses async
    #[message]
    pub async fn sleep(&self) {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    /// A message that sends a message to itself
    #[message]
    pub async fn inc_myself(&self) -> Result<(), SendError<i64>> {
        self.actor_ref().inc_async(1)
    }

    /// A message that panics
    #[message]
    pub fn force_panic(&self) {
        panic!("forced panic, don't worry this is correct and the actor will be restarted")
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), BoxError> {
    let counter = CounterActor::new();
    let actor = counter.spawn();

    // Increment
    actor.inc(2).await?;
    // Take borrowed string
    actor
        .borrow_data(&"hi".to_string())
        .await
        .map_err(SendError::reset)?;
    // Count should be 2
    assert_eq!(actor.count().await?, 2);

    // Trigger the actor to sleep for 100ms in the background
    actor.sleep_async()?;

    actor.error().await??;

    // If a panic occurs when running a message, we should receive an error that the actor was stopped
    assert_eq!(actor.force_panic().await, Err(SendError::ActorStopped));
    // An actor will also panic by default if an error is returned during an async message
    actor.error_async()?;
    // But we've implemented the `Actor::pre_restart` method to return `true`, so the actor will be restarted,
    // and new messages should be handled sucessfully, even with the state being preserved
    actor.inc(1).await?;
    assert_eq!(actor.count().await?, 3);

    // Stop the actor, dropping any pending messages
    actor.kill();
    // Await the actor to stop
    actor.wait_for_stop().await;
    // Any new messages should error since the actor is no longer running
    assert_eq!(actor.inc(1).await, Err(SendError::ActorNotRunning(1)));

    Ok(())
}
