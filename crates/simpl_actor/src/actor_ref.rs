use std::{
    collections::HashMap,
    fmt,
    sync::{Arc, Mutex},
};

use dyn_clone::DynClone;
use futures::{future::BoxFuture, stream::AbortHandle};
use tokio::sync::mpsc;

use crate::{err::SendError, internal::Signal, reason::ActorStopReason};

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

    /// Returns the current actor ref, if called from the running actor.
    fn current() -> Option<Self>;

    /// Links this actor with a child, notifying the child actor if the parent dies through
    /// [Actor::on_link_died](crate::actor::Actor::on_link_died), but not visa versa.
    fn link_child<R: ActorRef>(&self, child: &R);

    /// Unlinks the actor with a previously linked actor.
    fn unlink_child<R: ActorRef>(&self, child: &R);

    /// Links two actors with one another, notifying eachother if either actor dies through [Actor::on_link_died](crate::actor::Actor::on_link_died).
    ///
    /// This operation is atomic.
    fn link_together<R: ActorRef>(&self, actor_ref: &R);

    /// Unlinks two previously linked actors.
    ///
    /// This operation is atomic.
    fn unlink_together<R: ActorRef>(&self, actor_ref: &R);

    /// Notifies the actor that one of its links died.
    ///
    /// This is called automatically when an actor dies.
    fn notify_link_died(&self, id: u64, reason: ActorStopReason) -> Result<(), SendError>;

    /// Signals the actor to stop after processing all messages currently in its mailbox.
    ///
    /// This method sends a special stop message to the end of the actor's mailbox, ensuring
    /// that the actor will process all preceding messages before stopping. Any messages sent
    /// after this stop signal will be ignored and dropped. This approach allows for a graceful
    /// shutdown of the actor, ensuring all pending work is completed before termination.
    fn stop_gracefully(&self) -> Result<(), SendError>;

    /// Kills the actor immediately.
    ///
    /// This method aborts the actor immediately. Messages in the mailbox will be ignored and dropped.
    ///
    /// The actors on_stop hook will still be called.
    ///
    /// Note: If the actor is in the middle of processing a message, it will abort processing of that message.
    fn kill(&self);

    /// Waits for the actor to finish processing and stop.
    ///
    /// This method suspends execution until the actor has stopped, ensuring that any ongoing
    /// processing is completed and the actor has fully terminated. This is particularly useful
    /// in scenarios where it's necessary to wait for an actor to clean up its resources or
    /// complete its final tasks before proceeding.
    ///
    /// Note: This method does not initiate the stop process; it only waits for the actor to
    /// stop. You should signal the actor to stop using `stop_gracefully` or `kill`
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
}

trait Mailbox: DynClone + fmt::Debug + Send {
    fn closed(&self) -> BoxFuture<'_, ()>;
    fn is_closed(&self) -> bool;
    fn signal_stop(&self) -> Result<(), SendError>;
    fn signal_link_died(&self, id: u64, reason: ActorStopReason) -> Result<(), SendError>;
}

impl<T: Send> Mailbox for mpsc::UnboundedSender<Signal<T>> {
    fn closed(&self) -> BoxFuture<'_, ()> {
        Box::pin(self.closed())
    }

    fn is_closed(&self) -> bool {
        self.is_closed()
    }

    fn signal_stop(&self) -> Result<(), SendError> {
        self.send(Signal::Stop)
            .map_err(|err| SendError::from(err).reset())
    }

    fn signal_link_died(&self, id: u64, reason: ActorStopReason) -> Result<(), SendError> {
        self.send(Signal::LinkDied(id, reason))
            .map_err(|err| SendError::from(err).reset())
    }
}

dyn_clone::clone_trait_object!(Mailbox);

#[doc(hidden)]
#[derive(Clone, Debug)]
pub struct GenericActorRef {
    id: u64,
    mailbox: Box<dyn Mailbox>,
    abort_handle: AbortHandle,
    links: Arc<Mutex<HashMap<u64, GenericActorRef>>>,
}

impl GenericActorRef {
    pub fn from_parts<T: Send + 'static>(
        id: u64,
        mailbox: mpsc::UnboundedSender<Signal<T>>,
        abort_handle: AbortHandle,
        links: Arc<Mutex<HashMap<u64, GenericActorRef>>>,
    ) -> Self {
        GenericActorRef {
            id,
            mailbox: Box::new(mailbox),
            abort_handle,
            links,
        }
    }
}

impl Mailbox for GenericActorRef {
    fn closed(&self) -> BoxFuture<'_, ()> {
        self.mailbox.closed()
    }

    fn is_closed(&self) -> bool {
        self.mailbox.is_closed()
    }

    fn signal_stop(&self) -> Result<(), SendError> {
        self.mailbox.signal_stop()
    }

    fn signal_link_died(&self, id: u64, reason: ActorStopReason) -> Result<(), SendError> {
        self.mailbox.signal_link_died(id, reason)
    }
}

impl ActorRef for GenericActorRef {
    fn id(&self) -> u64 {
        self.id
    }

    fn is_alive(&self) -> bool {
        !self.is_closed()
    }

    fn current() -> Option<Self> {
        CURRENT_ACTOR.try_with(|actor_ref| actor_ref.clone()).ok()
    }

    fn link_child<R: ActorRef>(&self, child: &R) {
        if self.id == child.id() {
            return;
        }

        let child: GenericActorRef = child.clone().into_generic();
        self.links.lock().unwrap().insert(child.id, child);
    }

    fn unlink_child<R: ActorRef>(&self, child: &R) {
        if self.id == child.id() {
            return;
        }

        self.links.lock().unwrap().remove(&child.id());
    }

    fn link_together<R: ActorRef>(&self, actor_ref: &R) {
        if self.id == actor_ref.id() {
            return;
        }

        let actor_ref: GenericActorRef = actor_ref.clone().into_generic();
        let acotr_ref_links = actor_ref.links.clone();
        let (mut this_links, mut other_links) =
            (self.links.lock().unwrap(), acotr_ref_links.lock().unwrap());
        this_links.insert(actor_ref.id, actor_ref);
        other_links.insert(self.id, self.clone());
    }

    fn unlink_together<R: ActorRef>(&self, actor_ref: &R) {
        if self.id == actor_ref.id() {
            return;
        }

        let actor_ref: GenericActorRef = actor_ref.clone().into_generic();
        let (mut this_links, mut other_links) =
            (self.links.lock().unwrap(), actor_ref.links.lock().unwrap());
        this_links.remove(&actor_ref.id);
        other_links.remove(&self.id);
    }

    fn notify_link_died(&self, id: u64, reason: ActorStopReason) -> Result<(), SendError> {
        self.mailbox.signal_link_died(id, reason)
    }

    fn stop_gracefully(&self) -> Result<(), SendError> {
        self.mailbox.signal_stop()
    }

    fn kill(&self) {
        self.abort_handle.abort()
    }

    async fn wait_for_stop(&self) {
        self.mailbox.closed().await;
    }

    fn into_generic(self) -> GenericActorRef {
        self
    }
}
