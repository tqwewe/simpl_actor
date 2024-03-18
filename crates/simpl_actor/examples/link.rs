//! This example demonstrates how actors can be linked with one another.
//!
//! If actors are linked, then they will be notified if the other dies.

use futures::future::join;
use simpl_actor::*;

pub struct MyActor;

#[actor]
impl MyActor {
    #[message]
    pub fn force_panic(&self) {
        panic!("something went wrong");
    }
}

impl Actor for MyActor {
    type Ref = MyActorRef;

    async fn on_start(&mut self) -> Result<(), BoxError> {
        let id = self.actor_ref().id();
        println!("starting actor {id}");
        Ok(())
    }

    async fn on_stop(self, reason: ActorStopReason) -> Result<(), BoxError> {
        let id = self.actor_ref().id();
        println!("stopping actor {id} because {reason}");
        Ok(())
    }

    async fn on_link_died(
        &mut self,
        id: u64,
        reason: ActorStopReason,
    ) -> Result<Option<ActorStopReason>, BoxError> {
        match &reason {
            ActorStopReason::Normal => {
                println!("linked actor {id} stopped normally");
                Ok(None)
            }
            ActorStopReason::Killed
            | ActorStopReason::Panicked(_)
            | ActorStopReason::LinkDied { .. } => {
                println!("linked actor {id} stopped with error, stopping");
                Ok(Some(reason))
            }
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), SendError> {
    let parent = MyActor.spawn();
    let child_a = MyActor.spawn();
    let child_b = MyActor.spawn();
    parent.link_child(&child_b);

    let _ = child_a.force_panic().await;
    assert!(parent.is_alive());
    assert!(child_b.is_alive());

    let _ = parent.force_panic().await;
    assert!(!parent.is_alive());
    assert!(!child_b.is_alive());

    join(parent.wait_for_stop(), child_b.wait_for_stop()).await;

    Ok(())
}
