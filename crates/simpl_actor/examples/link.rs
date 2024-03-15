//! This example demonstrates how actors can be linked with one another.
//!
//! If actors are linked, then they will be notified if the other dies.

use std::convert::Infallible;

use futures::future::join;
use simpl_actor::*;

#[derive(Default)]
pub struct MyActor {
    id: u64,
}

#[actor]
impl MyActor {
    #[message]
    pub async fn spawn_linked(&self) -> MyActorRef {
        MyActor::default().spawn_link().await
    }

    #[message]
    pub fn force_panic(&self) {
        panic!("something went wrong");
    }
}

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
            ActorStopReason::Panicked(_) | ActorStopReason::LinkDied { .. } => {
                println!("linked actor {id} stopped with error, stopping");
                Ok(ShouldStop::Yes)
            }
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), ActorError> {
    let parent = MyActor::default().spawn();
    let child_a = MyActor::default().spawn();
    let child_b = MyActor::default().spawn();
    parent.link_child(&child_b).await;

    let _ = child_a.force_panic().await;
    assert!(parent.is_alive());
    assert!(child_b.is_alive());

    let _ = parent.force_panic().await;
    assert!(!parent.is_alive());
    assert!(!child_b.is_alive());

    join(parent.wait_for_stop(), child_b.wait_for_stop()).await;

    Ok(())
}
