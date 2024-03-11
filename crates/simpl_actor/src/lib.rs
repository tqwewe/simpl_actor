use std::{
    any::{self, Any},
    panic::AssertUnwindSafe,
};

use async_trait::async_trait;
use futures::{future::BoxFuture, FutureExt};
use tokio::sync::mpsc;
use tracing::debug;

pub use simpl_actor_macros::{actor, Actor};

#[async_trait]
pub trait Actor: Sized {
    type Error: Send;

    fn channel_size() -> usize {
        64
    }

    async fn pre_start(&mut self) -> Result<(), Self::Error> {
        debug!("starting actor {}", any::type_name::<Self>());
        Ok(())
    }

    async fn pre_restart(&mut self, _err: Box<dyn Any + Send>) -> Result<bool, Self::Error> {
        debug!("restarting actor {}", any::type_name::<Self>());
        Ok(false)
    }

    async fn pre_stop(self, _reason: ActorStopReason<Self::Error>) -> Result<(), Self::Error> {
        debug!("stopping actor {}", any::type_name::<Self>());
        Ok(())
    }
}

pub trait Spawn {
    type Ref;

    fn spawn(self) -> Self::Ref;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActorStopReason<E> {
    Normal,
    Panicked,
    StartFailed(E),
    RestartFailed(E),
}

#[doc(hidden)]
pub fn spawn_actor<A, M, F>(mut actor: A, buffer: usize, handle_messages: F) -> mpsc::Sender<M>
where
    A: Actor + Send + 'static,
    M: Send + 'static,
    F: for<'a> Fn(&'a mut A, &'a mut mpsc::Receiver<M>) -> BoxFuture<'a, ()> + Send + 'static,
{
    let (tx, mut rx) = mpsc::channel(buffer);
    tokio::spawn(async move {
        if let Err(err) = actor.pre_start().await {
            let _ = actor.pre_stop(ActorStopReason::StartFailed(err)).await;
            return;
        }

        let reason = loop {
            match AssertUnwindSafe(handle_messages(&mut actor, &mut rx))
                .catch_unwind()
                .await
            {
                Ok(()) => break ActorStopReason::Normal,
                Err(err) => match actor.pre_restart(err).await {
                    Ok(should_restart) => {
                        if should_restart == true {
                            continue;
                        }

                        break ActorStopReason::Panicked;
                    }
                    Err(err) => {
                        break ActorStopReason::RestartFailed(err);
                    }
                },
            }
        };

        if let Err(_) = actor.pre_stop(reason).await {
            return;
        }
    });

    tx
}
