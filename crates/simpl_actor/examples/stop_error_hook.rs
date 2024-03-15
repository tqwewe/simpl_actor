//! This example shows how to set custom hook for errors returns by `Actor::on_stop`.

use std::time::Duration;

use simpl_actor::*;

#[derive(Default)]
pub struct MyActor;

#[actor]
impl MyActor {}

impl Actor for MyActor {
    type Error = String;

    async fn on_stop(self, _reason: ActorStopReason) -> Result<(), Self::Error> {
        Err("error in on stop hook".to_string())
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), ActorError> {
    set_on_stop_error_hook(Box::new(|id, err| {
        Box::pin(async move {
            let msg = err.downcast::<String>().unwrap();
            println!("actor {id} errored: {msg}");
        })
    }))
    .await;

    let actor = MyActor.spawn();
    actor.stop_immediately()?;
    actor.wait_for_stop().await;

    tokio::time::sleep(Duration::from_millis(50)).await;

    Ok(())
}
