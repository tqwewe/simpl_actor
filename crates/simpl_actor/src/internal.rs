use std::{
    collections::HashMap,
    convert,
    panic::AssertUnwindSafe,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
};

use futures::{
    future::BoxFuture,
    stream::{AbortRegistration, Abortable},
    FutureExt,
};
use tokio::{
    sync::{mpsc, RwLock, Semaphore},
    task::JoinSet,
};

use crate::{
    actor::Actor,
    actor_ref::{ActorRef, GenericActorRef},
    err::PanicErr,
    reason::ActorStopReason,
};

static ACTOR_COUNTER: AtomicU64 = AtomicU64::new(0);

pub trait Message {
    fn is_read_only(&self) -> bool;
}

#[allow(missing_debug_implementations)]
pub enum Signal<M> {
    Message(Box<M>),
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
    A: Actor + Send + Sync + 'static,
    M: Message + Send + 'static,
    F: for<'a> Fn(&'a RwLock<A>, M) -> BoxFuture<'a, Option<ActorStopReason>>
        + Copy
        + Send
        + Sync
        + 'static,
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

    let actor = Arc::new(RwLock::new(actor));
    let mut concurrent_reads: JoinSet<Option<ActorStopReason>> = JoinSet::new();

    let reason = Abortable::new(
        run_abortable_future(&actor, mailbox_rx, handle_message, &mut concurrent_reads),
        abort_registration,
    )
    .await
    .unwrap_or(ActorStopReason::Killed);

    concurrent_reads.shutdown().await;

    if let Ok(mut links) = links.lock() {
        for (_, actor_ref) in links.drain() {
            let _ = actor_ref.notify_link_died(id, reason.clone());
        }
    }

    Arc::into_inner(actor)
        .expect("actor's arc contains other strong references")
        .into_inner()
        .on_stop(reason.clone())
        .await
        .unwrap();
}

async fn run_abortable_future<A, M, F>(
    actor: &Arc<RwLock<A>>,
    mut mailbox_rx: mpsc::UnboundedReceiver<Signal<M>>,
    handle_message: F,
    concurrent_reads: &mut JoinSet<Option<ActorStopReason>>,
) -> ActorStopReason
where
    A: Actor + Send + Sync + 'static,
    M: Message + Send + 'static,
    F: for<'a> Fn(&'a RwLock<A>, M) -> BoxFuture<'a, Option<ActorStopReason>>
        + Copy
        + Send
        + Sync
        + 'static,
{
    loop {
        let res = AssertUnwindSafe(recv_mailbox_loop(
            actor,
            &mut mailbox_rx,
            handle_message,
            concurrent_reads,
        ))
        .catch_unwind()
        .await;

        concurrent_reads.shutdown().await;

        match res {
            Ok(reason) => break reason,
            Err(err) => {
                let panic_err = PanicErr::new_boxed(err);
                match actor.write().await.on_panic(panic_err.clone()).await {
                    Ok(Some(reason)) => return reason,
                    Ok(None) => {}
                    Err(err) => break ActorStopReason::Panicked(PanicErr::new(err)),
                }
            }
        }
    }
}

async fn recv_mailbox_loop<A, M, F>(
    actor: &Arc<RwLock<A>>,
    mailbox_rx: &mut mpsc::UnboundedReceiver<Signal<M>>,
    handle_message: F,
    concurrent_reads: &mut JoinSet<Option<ActorStopReason>>,
) -> ActorStopReason
where
    A: Actor + Send + Sync + 'static,
    M: Message + Send + 'static,
    F: for<'a> Fn(&'a RwLock<A>, M) -> BoxFuture<'a, Option<ActorStopReason>>
        + Copy
        + Send
        + Sync
        + 'static,
{
    let semaphore = Arc::new(Semaphore::new(A::max_concurrent_reads()));
    macro_rules! wait_concurrent_reads {
        () => {
            while let Some(res) = concurrent_reads.join_next().await {
                match res {
                    Ok(Some(reason)) => return reason,
                    Ok(None) => {}
                    Err(err) => {
                        return ActorStopReason::Panicked(PanicErr::new_boxed(err.into_panic()))
                    }
                }
            }
        };
    }

    loop {
        tokio::select! {
            // biased;
            Some(res) = concurrent_reads.join_next() => {
                match res {
                    Ok(Some(reason)) => return reason,
                    Ok(None) => {}
                    Err(err) => {
                        return ActorStopReason::Panicked(PanicErr::new_boxed(err.into_panic()))
                    }
                }
            }
            signal = mailbox_rx.recv() => match signal {
                Some(Signal::Message(msg)) => {
                    if msg.is_read_only() {
                        let permit = semaphore.clone().acquire_owned().await;
                        let actor = Arc::clone(actor);
                        concurrent_reads.spawn(async move {
                            let _permit = permit;
                            handle_message(&actor, *msg).await
                        });
                    } else {
                        wait_concurrent_reads!();
                        if let Some(reason) = handle_message(&actor, *msg).await {
                            return reason;
                        }
                    }
                }
                Some(Signal::Stop) | None => {
                    wait_concurrent_reads!();
                    return ActorStopReason::Normal;
                }
                Some(Signal::LinkDied(id, reason)) => {
                    wait_concurrent_reads!();
                    match actor.write().await.on_link_died(id, reason.clone()).await {
                        Ok(Some(reason)) => return reason,
                        Ok(None) => {}
                        Err(err) => return ActorStopReason::Panicked(PanicErr::new(err)),
                    }
                }
            }
        }
    }
}
