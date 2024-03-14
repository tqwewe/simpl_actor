//! `simpl_actor` - A Rust library for actor-based concurrency, built on Tokio.
//!
//! # Features
//!
//! - Simple Actor Definition: Define actors with minimal code using Rust structs and macros.
//! - Automatic Message Handling: Leverages Rust's type system and async capabilities.
//! - Asynchronous & Synchronous Messaging: Flexible interaction patterns for actors.
//! - Lifecycle Management: Hooks for actor initialization, restart, and shutdown.
//!
//! # Quick Start
//!
//! ```
//! use simpl_actor::{actor, Actor, Spawn};
//!
//! #[derive(Actor, Default)]
//! pub struct CounterActor { count: i64 }
//!
//! #[actor]
//! impl CounterActor {
//!     #[message]
//!     pub fn inc(&mut self, amount: i64) { ... }
//!
//!     #[message]
//!     pub fn count(&self) -> i64 { ... }
//!
//!     #[message]
//!     pub async fn fetch(&self) -> String { ... }
//! }
//!
//! let actor = CounterActor::default().spawn();
//! actor.inc(1).await?;
//! actor.count().await?; // Returns 1
//! actor.fetch().await?;
//! ```
//!
//! # Messaging Variants
//!
//! Provides six variants for message handling, ranging from synchronous to non-blocking asynchronous methods, with or without timeouts.

#![warn(missing_docs)]
#![warn(clippy::all)]
#![warn(rust_2018_idioms)]
#![warn(missing_debug_implementations)]
#![deny(unused_must_use)]

use std::{
    any::{self, Any},
    collections::HashMap,
    fmt, mem,
    panic::AssertUnwindSafe,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

use async_trait::async_trait;
use futures::{future::BoxFuture, FutureExt};
use tokio::sync::{mpsc, Mutex, Notify};
use tracing::{debug, error};

pub use simpl_actor_macros::{actor, Actor};

/// Defines the core functionality for actors within an actor-based concurrency model.
///
/// Implementors of this trait can leverage asynchronous task execution,
/// lifecycle management hooks, and custom error handling.
///
/// This can be implemented with default behaviour using the [Actor](simpl_actor_macros::Actor) derive macro.
///
/// # Example
///
/// ```
/// #[async_trait]
/// impl Actor for MyActor {
///     type Error = ();
///
///     async fn on_start(&mut self, id: u64) -> Result<(), Self::Error> {
///         // actor is about to start up initially
///         Ok(())
///     }
///
///     async fn on_panic(&mut self, err: Box<dyn Any + Send>) -> Result<ShouldRestart, Self::Error> {
///         // actor panicked, return true if the actor should be restarted
///         Ok(true)
///     }
///
///     async fn on_stop(self, reason: ActorStopReason) -> Result<(), Self::Error> {
///         // actor is being stopped, any cleanup code can go here
///         Ok(())
///     }
///
///     async fn on_link_died(
///         &self,
///         id: u64,
///         reason: ActorStopReason,
///     ) -> Result<ShouldStop, Self::Error> {
///         match reason {
///             ActorStopReason::Normal => {
///                 debug!("linked actor {id} stopped normally");
///                 Ok(ShouldStop::No)
///             }
///             ActorStopReason::Panicked | ActorStopReason::LinkDied { .. } => {
///                 debug!("linked actor {id} stopped with error");
///                 Ok(ShouldStop::Yes)
///             }
///         }
///     }
/// }
/// ```
#[async_trait]
pub trait Actor: Spawn + Sized {
    /// The error type that can be returned from the actor's methods.
    type Error: Clone + fmt::Debug + Send + Sync + 'static;

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
    async fn on_panic(&mut self, _err: Box<dyn Any + Send>) -> Result<ShouldRestart, Self::Error> {
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
            ActorStopReason::Panicked | ActorStopReason::LinkDied { .. } => {
                debug!("linked actor {id} stopped with error");
                Ok(ShouldStop::Yes)
            }
        }
    }
}

/// Provides functionality to spawn actors and obtain references to them.
///
/// This trait defines how actors are spawned and allows for retrieving references to the actor.
/// Implementing this trait enables the creation and management of actor instances.
///
/// This trait is automatically implemented by the [`actor`] macro.
#[async_trait]
pub trait Spawn {
    /// The type of reference to the actor, allowing for message sending and interaction.
    type Ref: ActorRef;

    /// Spawns an actor, creating a new instance of it.
    ///
    /// This method takes ownership of the actor instance and starts its execution,
    /// returning a reference to the newly spawned actor. This reference can be used
    /// to send messages to the actor.
    ///
    /// The spawned actor is not linked to any others.
    ///
    /// # Returns
    /// A reference to the spawned actor, of the type specified by `Self::Ref`.
    fn spawn(self) -> Self::Ref;

    /// Spawns an actor with a bidirectional link between the current actor and the one being spawned.
    ///
    /// If either actor dies, [Actor::on_link_died] will be called on the other actor.
    async fn spawn_link(self) -> Self::Ref;

    /// Spawns an actor with a unidirectional link between the current actor and the child.
    ///
    /// If the current actor dies, [Actor::on_link_died] will be called on the spawned one,
    /// however if the spawned actor dies, [Actor::on_link_died] will not be called.
    async fn spawn_child(self) -> Self::Ref;

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
///
/// This trait is automatically implemented by the [`actor`] macro.
#[async_trait]
pub trait ActorRef: Clone + Send + Sync {
    /// Returns the actor identifier.
    fn id(&self) -> u64;

    /// Returns whether the actor is currently alive.
    fn is_alive(&self) -> bool;

    /// Links this actor with a child, notifying the child actor if the parent dies through
    /// [Actor::on_link_died], but not visa versa.
    async fn link_child<R: ActorRef>(&self, child: &R);

    /// Unlinks the actor with a previously linked actor.
    async fn unlink_child<R: ActorRef>(&self, child: &R);

    /// Links two actors with one another, notifying eachother if either actor dies through [Actor::on_link_died].
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

    #[doc(hidden)]
    fn into_generic(self) -> GenericActorRef;
    #[doc(hidden)]
    fn from_generic(actor_ref: GenericActorRef) -> Self;
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

/// Reason for an actor being stopped.
#[derive(Clone, Debug)]
pub enum ActorStopReason {
    /// Actor stopped normally.
    Normal,
    /// Actor panicked.
    Panicked,
    /// Link died.
    LinkDied {
        /// Actor ID.
        id: u64,
        /// Actor died reason.
        reason: Box<ActorStopReason>,
    },
}

impl fmt::Display for ActorStopReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ActorStopReason::Normal => write!(f, "actor stopped normally"),
            ActorStopReason::Panicked => write!(f, "actor panicked"),
            ActorStopReason::LinkDied { id, reason } => {
                write!(f, "link #{id} died with reason: {}", reason.as_ref())
            }
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

static ACTOR_COUNTER: AtomicU64 = AtomicU64::new(0);

#[doc(hidden)]
pub fn new_actor_id() -> u64 {
    ACTOR_COUNTER.fetch_add(1, Ordering::Relaxed)
}

tokio::task_local! {
    #[doc(hidden)]
    pub static CURRENT_ACTOR: GenericActorRef;
}

#[doc(hidden)]
#[derive(Clone, Debug)]
pub struct GenericActorRef {
    id: u64,
    mailbox: mpsc::Sender<Signal<()>>,
    stop_notify: Arc<Notify>,
    links: Arc<Mutex<HashMap<u64, GenericActorRef>>>,
}

impl GenericActorRef {
    pub fn current() -> Self {
        CURRENT_ACTOR.with(|actor_ref| actor_ref.clone())
    }

    pub fn try_current() -> Option<Self> {
        CURRENT_ACTOR.try_with(|actor_ref| actor_ref.clone()).ok()
    }

    pub fn from_parts<M>(
        id: u64,
        channel: mpsc::Sender<Signal<M>>,
        stop_notify: Arc<Notify>,
        links: Arc<Mutex<HashMap<u64, GenericActorRef>>>,
    ) -> Self {
        GenericActorRef {
            id,
            mailbox: unsafe { mem::transmute(channel) },
            stop_notify,
            links,
        }
    }

    pub fn into_parts<M>(
        self,
    ) -> (
        u64,
        mpsc::Sender<Signal<M>>,
        Arc<Notify>,
        Arc<Mutex<HashMap<u64, GenericActorRef>>>,
    ) {
        (
            self.id,
            unsafe { mem::transmute(self.mailbox) },
            self.stop_notify,
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

#[async_trait]
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

    fn stop_immediately(&self) {
        self.stop_notify.notify_waiters();
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

#[doc(hidden)]
#[allow(missing_debug_implementations)]
pub enum Signal<M> {
    Message(M),
    Stop,
    LinkDied(u64, ActorStopReason),
}

#[doc(hidden)]
pub async fn run_actor_lifecycle<A, M, F>(
    id: u64,
    mut actor: A,
    mut rx: mpsc::Receiver<Signal<M>>,
    stop_notify: Arc<Notify>,
    links: Arc<Mutex<HashMap<u64, GenericActorRef>>>,
    handle_message: F,
) where
    A: Actor + Send + Sync,
    F: for<'a> Fn(&'a mut A, M) -> BoxFuture<'a, ()>,
{
    if let Err(_err) = actor.on_start(id).await {
        let _ = actor.on_stop(ActorStopReason::Panicked).await;
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
                                    error!("on_link_died hook error: {err:?}");
                                    return ActorStopReason::Panicked;
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
            _ = stop_notify.notified() => {
                break ActorStopReason::Normal;
            }
            res = process_fut => {
                match res {
                    Ok(reason) => break reason,
                    Err(err) => match actor.on_panic(err).await {
                        Ok(ShouldRestart::Yes) => continue,
                        Ok(ShouldRestart::No) => break ActorStopReason::Panicked,
                        Err(_err) => {
                            break ActorStopReason::Panicked;
                        }
                    },
                }
            }
        }
    };

    if let Err(err) = actor.on_stop(reason.clone()).await {
        error!("on_stop hook error: {err:?}");
    }

    for (_, actor_ref) in links.lock().await.drain() {
        let _ = actor_ref.notify_link_died(id, reason.clone()).await;
    }
}
