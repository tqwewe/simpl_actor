use std::{
    collections::HashMap,
    convert,
    panic::AssertUnwindSafe,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

use futures::{
    future::BoxFuture,
    stream::{AbortRegistration, Abortable},
    FutureExt,
};
use tokio::sync::{mpsc, Mutex};

use crate::{
    actor::Actor,
    actor_ref::{ActorRef, GenericActorRef},
    err::PanicErr,
    reason::ActorStopReason,
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
    mailbox_rx: mpsc::UnboundedReceiver<Signal<M>>,
    abort_registration: AbortRegistration,
    links: Arc<Mutex<HashMap<u64, GenericActorRef>>>,
    handle_message: F,
) where
    A: Actor + Send,
    F: for<'a> Fn(&'a mut A, M) -> BoxFuture<'a, Option<ActorStopReason>>,
{
    let start_res = AssertUnwindSafe(actor.on_start())
        .catch_unwind()
        .await
        .map(|res| res.map_err(|err| PanicErr::new(err)))
        .map_err(PanicErr::new_boxed)
        .and_then(convert::identity);
    if let Err(err) = start_res {
        actor.on_stop(ActorStopReason::Panicked(err)).await.unwrap();
        return;
    }

    let reason = Abortable::new(
        run_abortable_future(&mut actor, mailbox_rx, handle_message),
        abort_registration,
    )
    .await
    .unwrap_or(ActorStopReason::Killed);

    for (_, actor_ref) in links.lock().await.drain() {
        let _ = actor_ref.notify_link_died(id, reason.clone());
    }

    actor.on_stop(reason.clone()).await.unwrap();
}

async fn run_abortable_future<A, M, F>(
    mut actor: &mut A,
    mut mailbox_rx: mpsc::UnboundedReceiver<Signal<M>>,
    handle_message: F,
) -> ActorStopReason
where
    A: Actor + Send,
    F: for<'a> Fn(&'a mut A, M) -> BoxFuture<'a, Option<ActorStopReason>>,
{
    loop {
        let res = AssertUnwindSafe({
            let actor = &mut actor;
            let mailbox_rx = &mut mailbox_rx;
            let handle_message = &handle_message;
            async move {
                while let Some(signal) = mailbox_rx.recv().await {
                    match signal {
                        Signal::Message(msg) => {
                            if let Some(reason) = handle_message(actor, msg).await {
                                return reason;
                            }
                        }
                        Signal::Stop => return ActorStopReason::Normal,
                        Signal::LinkDied(id, reason) => {
                            match actor.on_link_died(id, reason.clone()).await {
                                Ok(Some(reason)) => return reason,
                                Ok(None) => {}
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
        .catch_unwind()
        .await;
        match res {
            Ok(reason) => break reason,
            Err(err) => {
                let panic_err = PanicErr::new_boxed(err);
                match actor.on_panic(panic_err.clone()).await {
                    Ok(Some(reason)) => return reason,
                    Ok(None) => {}
                    Err(err) => break ActorStopReason::Panicked(PanicErr::new(err)),
                }
            }
        }
    }
}
