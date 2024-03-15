use std::{
    any::Any,
    collections::HashMap,
    convert,
    panic::AssertUnwindSafe,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

use futures::{future::BoxFuture, FutureExt, TryFutureExt};
use tokio::sync::{mpsc, watch, Mutex};

use crate::{
    actor::{Actor, ShouldRestart, ShouldStop},
    actor_ref::{ActorRef, GenericActorRef},
    err::PanicErr,
    reason::ActorStopReason,
    stop_error_hook::StopErrorHook,
};

static ACTOR_COUNTER: AtomicU64 = AtomicU64::new(0);

#[allow(missing_debug_implementations)]
pub enum Signal<M> {
    Message(M),
    Stop,
    LinkDied(u64, ActorStopReason),
}

pub fn new_actor_id() -> u64 {
    ACTOR_COUNTER.fetch_add(1, Ordering::Relaxed)
}

pub async fn run_actor_lifecycle<A, M, F>(
    id: u64,
    mut actor: A,
    mut rx: mpsc::Receiver<Signal<M>>,
    mut stop_rx: watch::Receiver<()>,
    links: Arc<Mutex<HashMap<u64, GenericActorRef>>>,
    handle_message: F,
) where
    A: Actor + Send,
    F: for<'a> Fn(&'a mut A, M) -> BoxFuture<'a, ()>,
{
    if let Err(err) = AssertUnwindSafe(actor.on_start(id)).catch_unwind().await {
        let on_stop_res = AssertUnwindSafe(
            actor
                .on_stop(ActorStopReason::Panicked(PanicErr::new_boxed(err)))
                .map_err(|err| Box::new(err) as Box<dyn Any + Send>),
        )
        .catch_unwind()
        .await
        .and_then(convert::identity);
        if let Err(err) = on_stop_res {
            StopErrorHook::call_hook(id, Box::new(err)).await;
        }
        return;
    }

    let reason = loop {
        let process_fut = AssertUnwindSafe({
            let actor = &mut actor;
            let rx = &mut rx;
            let handle_message = &handle_message;
            async move {
                while let Some(signal) = rx.recv().await {
                    match signal {
                        Signal::Message(msg) => handle_message(actor, msg).await,
                        Signal::Stop => break,
                        Signal::LinkDied(id, reason) => {
                            match actor.on_link_died(id, reason.clone()).await {
                                Ok(ShouldStop::Yes) => {
                                    return ActorStopReason::LinkDied {
                                        id,
                                        reason: Box::new(reason),
                                    }
                                }
                                Ok(ShouldStop::No) => {}
                                Err(err) => {
                                    return ActorStopReason::Panicked(PanicErr::new(err));
                                }
                            }
                        }
                    }
                }

                ActorStopReason::Normal
            }
        })
        .catch_unwind();
        tokio::select! {
            biased;
            _ = stop_rx.changed() => {
                break ActorStopReason::Normal;
            }
            res = process_fut => {
                match res {
                    Ok(reason) => break reason,
                    Err(err) => {
                        let panic_err = PanicErr::new_boxed(err);
                        match actor.on_panic(panic_err.clone()).await {
                            Ok(ShouldRestart::Yes) => continue,
                            Ok(ShouldRestart::No) => break ActorStopReason::Panicked(panic_err),
                            Err(err) => break ActorStopReason::Panicked(PanicErr::new(err)),
                        }
                    },
                }
            }
        }
    };

    let on_stop_res = AssertUnwindSafe(
        actor
            .on_stop(reason.clone())
            .map_err(|err| Box::new(err) as Box<dyn Any + Send>),
    )
    .catch_unwind()
    .await
    .and_then(convert::identity);
    if let Err(err) = on_stop_res {
        StopErrorHook::call_hook(id, err).await;
    }

    for (_, actor_ref) in links.lock().await.drain() {
        let _ = actor_ref.notify_link_died(id, reason.clone()).await;
    }
}
