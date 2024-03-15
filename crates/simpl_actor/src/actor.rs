use std::any;

use tracing::debug;

use crate::{err::PanicErr, reason::ActorStopReason};

/// Defines the core functionality for actors within an actor-based concurrency model.
///
/// Implementors of this trait can leverage asynchronous task execution,
/// lifecycle management hooks, and custom error handling.
///
/// This can be implemented with default behaviour using the [Actor](simpl_actor_macros::Actor) derive macro.
#[allow(async_fn_in_trait)]
pub trait Actor: Sized {
    /// The error type that can be returned from the actor's methods.
    type Error: Send + 'static;

    /// Specifies the default size of the actor's mailbox.
    ///
    /// This method can be overridden to change the size of the mailbox, affecting
    /// how many messages can be queued before backpressure is applied.
    ///
    /// # Returns
    /// The size of the mailbox. The default is 64.
    fn mailbox_size() -> usize {
        64
    }

    /// Hook that is called before the actor starts processing messages.
    ///
    /// This asynchronous method allows for initialization tasks to be performed
    /// before the actor starts receiving messages.
    ///
    /// # Returns
    /// A result indicating successful initialization or an error if initialization fails.
    async fn on_start(&mut self, id: u64) -> Result<(), Self::Error> {
        debug!("starting actor #{id} {}", any::type_name::<Self>());
        Ok(())
    }

    /// Hook that is called when an actor panicked.
    ///
    /// This method provides an opportunity to clean up or reset state.
    /// It can also determine whether the actor should actually be restarted
    /// based on the nature of the error.
    ///
    /// # Parameters
    /// - `err`: The error that occurred.
    ///
    /// # Returns
    /// Whether the actor should be restarted or not.
    async fn on_panic(&mut self, _err: PanicErr) -> Result<ShouldRestart, Self::Error> {
        Ok(ShouldRestart::No)
    }

    /// Hook that is called before the actor is stopped.
    ///
    /// This method allows for cleanup and finalization tasks to be performed before the
    /// actor is fully stopped. It can be used to release resources, notify other actors,
    /// or complete any final tasks.
    ///
    /// # Parameters
    /// - `reason`: The reason why the actor is being stopped.
    ///
    /// # Returns
    /// A result indicating the successful cleanup or an error if the cleanup process fails.
    async fn on_stop(self, _reason: ActorStopReason) -> Result<(), Self::Error> {
        debug!("stopping actor {}", any::type_name::<Self>());
        Ok(())
    }

    /// Hook that is called when a linked actor dies.
    ///
    /// By default, the current actor will be stopped if the reason is anything other than normal.
    ///
    /// # Returns
    /// Whether the actor should be stopped or not.
    async fn on_link_died(
        &self,
        id: u64,
        reason: ActorStopReason,
    ) -> Result<ShouldStop, Self::Error> {
        match reason {
            ActorStopReason::Normal => {
                debug!("linked actor {id} stopped normally");
                Ok(ShouldStop::No)
            }
            ActorStopReason::Panicked(_) => {
                debug!("linked actor {id} panicked");
                Ok(ShouldStop::Yes)
            }
            ActorStopReason::LinkDied {
                id: other_actor, ..
            } => {
                debug!("linked actor {id} died due to actor {other_actor} dying");
                Ok(ShouldStop::Yes)
            }
        }
    }
}

/// Enum indiciating if an actor should be restarted or not after panicking.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShouldRestart {
    /// The actor should be restarted.
    Yes,
    /// The actor should stop.
    No,
}

/// Enum indiciating if an actor should be stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShouldStop {
    /// The actor should be stopped.
    Yes,
    /// The actor should not be stopped.
    No,
}
