use std::{any, borrow::Cow, error::Error};

use tracing::{debug, enabled, warn, Level};

use crate::{err::PanicErr, reason::ActorStopReason, ActorRef, GenericActorRef};

/// A boxed dyn std Error used in actor hooks.
pub type BoxError = Box<dyn Error + Send + Sync + 'static>;

/// Defines the core functionality for actors within an actor-based concurrency model.
///
/// Implementors of this trait can leverage asynchronous task execution,
/// lifecycle management hooks, and custom error handling.
///
/// This can be implemented with default behaviour using the [Actor](simpl_actor_macros::Actor) derive macro.
///
/// Methods in this trait that return `BoxError` will stop the actor with the reason `ActorReason::Panicked` with the error.
#[allow(async_fn_in_trait)]
pub trait Actor: Sized {
    /// Actor ref type.
    type Ref: ActorRef;

    /// Actor name, useful for logging.
    fn name(&self) -> Cow<'_, str> {
        Cow::Borrowed(any::type_name::<Self>())
    }

    /// Retrieves a reference to the current actor.
    ///
    /// # Panics
    ///
    /// This function will panic if called outside the scope of an actor.
    ///
    /// # Returns
    /// A reference to the actor of type `Self::Ref`.
    fn actor_ref(&self) -> Self::Ref {
        match Self::try_actor_ref() {
            Some(actor_ref) => actor_ref,
            None => panic!("actor_ref called outside the scope of an actor"),
        }
    }

    /// Retrieves a reference to the current actor, if available.
    ///
    /// # Returns
    /// An `Option` containing a reference to the actor of type `Self::Ref` if available,
    /// or `None` if the actor reference is not available.
    fn try_actor_ref() -> Option<Self::Ref> {
        GenericActorRef::try_current().map(Self::Ref::from_generic)
    }

    /// Hook that is called before the actor starts processing messages.
    ///
    /// This asynchronous method allows for initialization tasks to be performed
    /// before the actor starts receiving messages.
    ///
    /// # Returns
    /// A result indicating successful initialization or an error if initialization fails.
    async fn on_start(&mut self) -> Result<(), BoxError> {
        if enabled!(Level::DEBUG) {
            let id = self.actor_ref().id();
            let name = self.name();
            debug!("starting actor {name} ({id})");
        }

        Ok(())
    }

    /// Hook that is called when an actor panicked or returns an error during an async message.
    ///
    /// This method provides an opportunity to clean up or reset state.
    /// It can also determine whether the actor should be killed or if it should continue processing messages by returning `None`.
    ///
    /// # Parameters
    /// - `err`: The error that occurred.
    ///
    /// # Returns
    /// Whether the actor should continue processing, or be stopped by returning a stop reason.
    async fn on_panic(&mut self, err: PanicErr) -> Result<Option<ActorStopReason>, BoxError> {
        Ok(Some(ActorStopReason::Panicked(err)))
    }

    /// Hook that is called before the actor is stopped.
    ///
    /// This method allows for cleanup and finalization tasks to be performed before the
    /// actor is fully stopped. It can be used to release resources, notify other actors,
    /// or complete any final tasks.
    ///
    /// # Parameters
    /// - `reason`: The reason why the actor is being stopped.
    async fn on_stop(self, reason: ActorStopReason) -> Result<(), BoxError> {
        if enabled!(Level::WARN) {
            let id = self.actor_ref().id();
            let name = self.name();
            match reason {
                ActorStopReason::Normal => {
                    debug!("actor {name} ({id}) stopped normally");
                }
                ActorStopReason::Killed => {
                    debug!("actor {name} ({id}) was killed");
                }
                ActorStopReason::Panicked(err) => {
                    match err.with_str(|err| {
                        warn!("actor {name} ({id}) panicked: {err}");
                    }) {
                        Ok(Some(_)) => {}
                        _ => {
                            warn!("actor {name} ({id}) panicked");
                        }
                    }
                }
                ActorStopReason::LinkDied {
                    id: link_id,
                    reason,
                } => {
                    warn!("actor {name} ({id}) was killed due to link ({link_id}) died with reason: {reason}");
                }
            }
        }

        Ok(())
    }

    /// Hook that is called when a linked actor dies.
    ///
    /// By default, the current actor will be stopped if the reason is anything other than normal.
    ///
    /// # Returns
    /// Whether the actor should continue processing, or be stopped by returning a stop reason.
    async fn on_link_died(
        &mut self,
        #[allow(unused)] id: u64,
        reason: ActorStopReason,
    ) -> Result<Option<ActorStopReason>, BoxError> {
        match &reason {
            ActorStopReason::Normal => Ok(None),
            ActorStopReason::Killed
            | ActorStopReason::Panicked(_)
            | ActorStopReason::LinkDied { .. } => Ok(Some(reason)),
        }
    }
}
