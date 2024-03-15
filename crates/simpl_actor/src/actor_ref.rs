use std::{collections::HashMap, mem, sync::Arc};

use tokio::sync::{mpsc, watch, Mutex};

use crate::{err::ActorError, internal::Signal, reason::ActorStopReason};

tokio::task_local! {
    #[doc(hidden)]
    pub static CURRENT_ACTOR: GenericActorRef;
}

/// Provides functionality to stop and wait for an actor to complete based on an actor ref.
///
/// This trait is automatically implemented by the [`#[actor]`](macro@crate::actor) macro.
#[allow(async_fn_in_trait)]
pub trait ActorRef: Clone {
    /// Returns the actor identifier.
    fn id(&self) -> u64;

    /// Returns whether the actor is currently alive.
    fn is_alive(&self) -> bool;

    /// Links this actor with a child, notifying the child actor if the parent dies through
    /// [Actor::on_link_died](crate::actor::Actor::on_link_died), but not visa versa.
    async fn link_child<R: ActorRef>(&self, child: &R);

    /// Unlinks the actor with a previously linked actor.
    async fn unlink_child<R: ActorRef>(&self, child: &R);

    /// Links two actors with one another, notifying eachother if either actor dies through [Actor::on_link_died](crate::actor::Actor::on_link_died).
    ///
    /// This operation is atomic.
    async fn link_together<R: ActorRef>(&self, actor_ref: &R);

    /// Unlinks two previously linked actors.
    ///
    /// This operation is atomic.
    async fn unlink_together<R: ActorRef>(&self, actor_ref: &R);

    /// Notifies the actor that one of its links died.
    ///
    /// This is called automatically when an actor dies.
    async fn notify_link_died(&self, id: u64, reason: ActorStopReason) -> Result<(), ActorError>;

    /// Signals the actor to stop after processing all messages currently in its mailbox.
    ///
    /// This method sends a special stop message to the end of the actor's mailbox, ensuring
    /// that the actor will process all preceding messages before stopping. Any messages sent
    /// after this stop signal will be ignored and dropped. This approach allows for a graceful
    /// shutdown of the actor, ensuring all pending work is completed before termination.
    async fn stop_gracefully(&self) -> Result<(), ActorError>;

    /// Signals the actor to stop immediately, bypassing its mailbox.
    ///
    /// This method instructs the actor to terminate as soon as it finishes processing the
    /// current message, if any. Messages in the mailbox that have not yet been processed
    /// will be ignored and dropped. This method is useful for immediate shutdown scenarios
    /// where waiting for the mailbox to empty is not feasible or desired.
    ///
    /// Note: If the actor is in the middle of processing a message, it will complete that
    /// message before stopping.
    fn stop_immediately(&self) -> Result<(), ActorError>;

    /// Waits for the actor to finish processing and stop.
    ///
    /// This method suspends execution until the actor has stopped, ensuring that any ongoing
    /// processing is completed and the actor has fully terminated. This is particularly useful
    /// in scenarios where it's necessary to wait for an actor to clean up its resources or
    /// complete its final tasks before proceeding.
    ///
    /// Note: This method does not initiate the stop process; it only waits for the actor to
    /// stop. You should signal the actor to stop using `stop_gracefully` or `stop_immediately`
    /// before calling this method.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// // Assuming `actor.stop_gracefully().await` has been called earlier
    /// actor.wait_for_stop().await;
    /// ```
    async fn wait_for_stop(&self);

    #[doc(hidden)]
    fn into_generic(self) -> GenericActorRef;
    #[doc(hidden)]
    fn from_generic(actor_ref: GenericActorRef) -> Self;
}

#[doc(hidden)]
#[derive(Clone, Debug)]
pub struct GenericActorRef {
    id: u64,
    mailbox: mpsc::Sender<Signal<()>>,
    stop_tx: Arc<watch::Sender<()>>,
    links: Arc<Mutex<HashMap<u64, GenericActorRef>>>,
}

impl GenericActorRef {
    pub fn current() -> Self {
        CURRENT_ACTOR.with(|actor_ref| actor_ref.clone())
    }

    pub fn try_current() -> Option<Self> {
        CURRENT_ACTOR.try_with(|actor_ref| actor_ref.clone()).ok()
    }

    pub unsafe fn from_parts<M>(
        id: u64,
        channel: mpsc::Sender<Signal<M>>,
        stop_tx: Arc<watch::Sender<()>>,
        links: Arc<Mutex<HashMap<u64, GenericActorRef>>>,
    ) -> Self {
        GenericActorRef {
            id,
            mailbox: mem::transmute(channel),
            stop_tx,
            links,
        }
    }

    pub unsafe fn into_parts<M>(
        self,
    ) -> (
        u64,
        mpsc::Sender<Signal<M>>,
        Arc<watch::Sender<()>>,
        Arc<Mutex<HashMap<u64, GenericActorRef>>>,
    ) {
        (
            self.id,
            mem::transmute(self.mailbox),
            self.stop_tx,
            self.links,
        )
    }

    async fn signal(&self, signal: Signal<()>) -> Result<(), ActorError> {
        self.mailbox
            .send(signal)
            .await
            .map_err(|_| ActorError::ActorNotRunning(()))
    }
}

impl ActorRef for GenericActorRef {
    fn id(&self) -> u64 {
        self.id
    }

    fn is_alive(&self) -> bool {
        !self.mailbox.is_closed()
    }

    async fn link_child<R: ActorRef>(&self, child: &R) {
        if self.id == child.id() {
            return;
        }

        let child: GenericActorRef = child.clone().into_generic();
        self.links.lock().await.insert(child.id, child);
    }

    async fn unlink_child<R: ActorRef>(&self, child: &R) {
        if self.id == child.id() {
            return;
        }

        self.links.lock().await.remove(&child.id());
    }

    async fn link_together<R: ActorRef>(&self, actor_ref: &R) {
        if self.id == actor_ref.id() {
            return;
        }

        let actor_ref: GenericActorRef = actor_ref.clone().into_generic();
        let acotr_ref_links = actor_ref.links.clone();
        let (mut this_links, mut other_links) =
            tokio::join!(self.links.lock(), acotr_ref_links.lock());
        this_links.insert(actor_ref.id, actor_ref);
        other_links.insert(self.id, self.clone());
    }

    async fn unlink_together<R: ActorRef>(&self, actor_ref: &R) {
        if self.id == actor_ref.id() {
            return;
        }

        let actor_ref: GenericActorRef = actor_ref.clone().into_generic();
        let (mut this_links, mut other_links) =
            tokio::join!(self.links.lock(), actor_ref.links.lock());
        this_links.remove(&actor_ref.id);
        other_links.remove(&self.id);
    }

    async fn notify_link_died(&self, id: u64, reason: ActorStopReason) -> Result<(), ActorError> {
        self.signal(Signal::LinkDied(id, reason)).await
    }

    async fn stop_gracefully(&self) -> Result<(), ActorError> {
        self.signal(Signal::Stop).await
    }

    fn stop_immediately(&self) -> Result<(), ActorError> {
        self.stop_tx
            .send(())
            .map_err(|_| ActorError::ActorNotRunning(()))
    }

    async fn wait_for_stop(&self) {
        self.mailbox.closed().await;
    }

    fn into_generic(self) -> GenericActorRef {
        self
    }

    fn from_generic(actor_ref: GenericActorRef) -> Self {
        actor_ref
    }
}
