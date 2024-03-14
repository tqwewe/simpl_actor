use std::convert::Infallible;

use async_trait::async_trait;
use futures::future::join;
use simpl_actor::{actor, Actor, ActorError, ActorRef, ActorStopReason, ShouldStop, Spawn};

#[derive(Default)]
pub struct MyActor {
    id: u64,
}

#[actor]
impl MyActor {
    #[message]
    pub async fn spawn_linked(&self) -> Result<MyActorRef, ActorError> {
        MyActor::default().spawn_link().await
    }

    #[message]
    pub fn force_panic(&self) {
        panic!("something went wrong");
    }
}

#[async_trait]
impl Actor for MyActor {
    type Error = Infallible;

    async fn on_start(&mut self, id: u64) -> Result<(), Self::Error> {
        println!("starting #{id}");
        self.id = id;
        Ok(())
    }

    async fn on_stop(self, reason: ActorStopReason) -> Result<(), Self::Error> {
        println!("stopping #{} because {reason}", self.id);
        Ok(())
    }

    async fn on_link_died(
        &self,
        id: u64,
        reason: ActorStopReason,
    ) -> Result<ShouldStop, Self::Error> {
        match reason {
            ActorStopReason::Normal => {
                println!("linked actor {id} stopped normally");
                Ok(ShouldStop::No)
            }
            ActorStopReason::Panicked | ActorStopReason::LinkDied { .. } => {
                println!("linked actor {id} stopped with error, stopping");
                Ok(ShouldStop::Yes)
            }
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), ActorError> {
    let one = MyActor::default().spawn();
    let two = one.spawn_linked().await??;
    let _ = two.force_panic().await;

    join(one.wait_for_stop(), two.wait_for_stop()).await;

    Ok(())
}
