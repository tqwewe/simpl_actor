//! `simpl_actor` - A Rust library for actor-based concurrency, built on Tokio.
//!
//! # Features
//! - Simple Actor Definition: Define actors with minimal code using Rust structs and macros.
//! - Automatic Message Handling: Leverages Rust's type system and async capabilities.
//! - Asynchronous & Synchronous Messaging: Flexible interaction patterns for actors.
//! - Lifecycle Management: Hooks for actor initialization, restart, and shutdown.
//!
//! # Quick Start
//! ```
//! use simpl_actor::{actor, Actor, Spawn};
//!
//! #[derive(Actor)]
//! pub struct CounterActor { count: i64, }
//!
//! #[actor]
//! impl CounterActor {
//!     pub fn new() -> Self { CounterActor { count: 0 } }
//!     #[message] pub fn inc(&mut self, amount: i64) {}
//!     #[message] pub fn dec(&mut self, amount: i64) {}
//!     #[message] pub fn count(&self) -> i64 {}
//! }
//! ```
//!
//! # Messaging Variants
//! Provides six variants for message handling, ranging from synchronous to non-blocking asynchronous methods, with or without timeouts.
//!
//! # Contributing
//! Contributions are welcome. Submit pull requests, create issues, or suggest improvements.
//!
//! # License
//! Dual-licensed under MIT or Apache License, Version 2.0. Choose the license that best suits your project needs.

#![warn(missing_docs)]
#![warn(clippy::all)]
#![warn(rust_2018_idioms)]
#![warn(missing_debug_implementations)]
#![deny(unused_must_use)]

use std::{
    any::{self, Any},
    fmt,
    panic::AssertUnwindSafe,
    sync::Arc,
};

use async_trait::async_trait;
use futures::{future::BoxFuture, FutureExt};
use tokio::sync::{mpsc, Notify};
use tracing::debug;

pub use simpl_actor_macros::{actor, Actor};

/// Defines the core functionality for actors within an actor-based concurrency model.
///
/// Implementors of this trait can leverage asynchronous task execution,
/// lifecycle management hooks, and custom error handling.
#[async_trait]
pub trait Actor: Sized {
    /// The error type that can be returned from the actor's methods.
    type Error: Send;

    /// Specifies the default size of the actor's message channel.
    ///
    /// This method can be overridden to change the size of the mailbox, affecting
    /// how many messages can be queued before backpressure is applied.
    ///
    /// # Returns
    /// The size of the message channel. The default is 64.
    fn channel_size() -> usize {
        64
    }

    /// Hook that is called before the actor starts processing messages.
    ///
    /// This asynchronous method allows for initialization tasks to be performed
    /// before the actor starts receiving messages.
    ///
    /// # Returns
    /// A result indicating successful initialization or an error if initialization fails.
    async fn pre_start(&mut self) -> Result<(), Self::Error> {
        debug!("starting actor {}", any::type_name::<Self>());
        Ok(())
    }

    /// Hook that is called when an actor is about to be restarted after an error.
    ///
    /// This method provides an opportunity to clean up or reset state before the actor
    /// is restarted. It can also determine whether the actor should actually be restarted
    /// based on the nature of the error.
    ///
    /// # Parameters
    /// - `err`: The error that caused the actor to be restarted.
    ///
    /// # Returns
    /// A boolean indicating whether the actor should be restarted (`true`) or not (`false`).
    async fn pre_restart(&mut self, _err: Box<dyn Any + Send>) -> Result<bool, Self::Error> {
        Ok(false)
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
    async fn pre_stop(self, _reason: ActorStopReason<Self::Error>) -> Result<(), Self::Error> {
        debug!("stopping actor {}", any::type_name::<Self>());
        Ok(())
    }
}

/// Provides functionality to spawn actors and obtain references to them.
///
/// This trait defines how actors are spawned and allows for retrieving references to the actor.
/// Implementing this trait enables the creation and management of actor instances.
///
/// This trait is automatically implemented by the [`actor`] macro.
pub trait Spawn {
    /// The type of reference to the actor, allowing for message sending and interaction.
    type Ref;

    /// Spawns an actor, creating a new instance of it.
    ///
    /// This method takes ownership of the actor instance and starts its execution,
    /// returning a reference to the newly spawned actor. This reference can be used
    /// to send messages to the actor.
    ///
    /// # Returns
    /// A reference to the spawned actor, of the type specified by `Self::Ref`.
    fn spawn(self) -> Self::Ref;

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
    fn try_actor_ref() -> Option<Self::Ref>;
}

/// Provides functionality to stop and wait for an actor to complete based on an actor ref.
#[async_trait]
pub trait ActorRef {
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
    fn stop_immediately(&self);

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
}

/// Reason for an actor being stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActorStopReason<E> {
    /// Actor stopped normally.
    Normal,
    /// Actor panicked.
    Panicked,
    /// Actor failed to start.
    StartFailed(E),
    /// Actor failed to restart.
    RestartFailed(E),
}

impl<E: fmt::Display> fmt::Display for ActorStopReason<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ActorStopReason::Normal => write!(f, "actor stopped normally"),
            ActorStopReason::Panicked => write!(f, "actor panicked"),
            ActorStopReason::StartFailed(err) => write!(f, "actor failed to start: {err}"),
            ActorStopReason::RestartFailed(err) => write!(f, "actor failed to restart: {err}"),
        }
    }
}

/// Error that can occur when sending a message to an actor.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ActorError<E = ()> {
    /// The actor isn't running.
    ActorNotRunning(E),
    /// The actor panicked or was stopped.
    ActorStopped,
    /// The actors mailbox is full and sending would require blocking.
    MailboxFull(E),
    /// The actor's mailbox remained full, and the timeout elapsed.
    Timeout(E),
}

impl<E> fmt::Debug for ActorError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ActorError::ActorNotRunning(_) => {
                write!(f, "ActorNotRunning")
            }
            ActorError::ActorStopped => write!(f, "ActorStopped"),
            ActorError::MailboxFull(_) => {
                write!(f, "MailboxFull")
            }
            ActorError::Timeout(_) => {
                write!(f, "Timeout")
            }
        }
    }
}

impl<E> fmt::Display for ActorError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ActorError::ActorNotRunning(_) => write!(f, "actor not running"),
            ActorError::ActorStopped => write!(f, "actor stopped"),
            ActorError::MailboxFull(_) => write!(f, "no available capacity in mailbox"),
            ActorError::Timeout(_) => write!(f, "timed out waiting for space in mailbox"),
        }
    }
}

impl<E: fmt::Debug> std::error::Error for ActorError<E> {}

#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Signal<M> {
    Message(M),
    Stop,
}

#[doc(hidden)]
pub async fn run_actor_lifecycle<A, M, F>(
    mut actor: A,
    mut rx: mpsc::Receiver<Signal<M>>,
    stop_notify: Arc<Notify>,
    handle_messages: F,
) where
    A: Actor + Send,
    F: for<'a> Fn(&'a mut A, &'a mut mpsc::Receiver<Signal<M>>) -> BoxFuture<'a, ()>,
{
    if let Err(err) = actor.pre_start().await {
        let _ = actor.pre_stop(ActorStopReason::StartFailed(err)).await;
        return;
    }

    let reason = loop {
        let nest_signal_fut = AssertUnwindSafe(handle_messages(&mut actor, &mut rx)).catch_unwind();
        tokio::select! {
            biased;
            _ = stop_notify.notified() => {
                break ActorStopReason::Normal;
            }
            res = nest_signal_fut => {
                match res {
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
            }
        }
    };

    if let Err(_) = actor.pre_stop(reason).await {
        return;
    }
}
