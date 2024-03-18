//! `simpl_actor` - A Rust library for actor-based concurrency, built on Tokio.
//!
//! # Features
//!
//! - **Simple Actor Definition:** Define actors with minimal code using Rust structs and macros.
//! - **Automatic Message Handling:** Leverages Rust's type system and async capabilities.
//! - **Asynchronous & Synchronous Messaging:** Flexible interaction patterns for actors.
//! - **Lifecycle Management:** Hooks for actor initialization, restart, and shutdown.
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
//!     pub fn inc(&mut self, amount: i64) { ... }
//!
//!     #[message(infallible)]
//!     pub fn count(&self) -> i64 { ... }
//! }
//!
//! let counter = CounterActor::new();
//! let actor = counter.spawn();
//!
//! actor.inc(2).await?;
//! actor.dec(1).await?;
//! let count = actor.count().await?;
//! ```
//!
//! # Messaging Variants
//!
//! When you define a message in `simpl_actor`, two variants of the message handling function
//! are automatically for syncronous and asyncronous processing:
//!
//! ```
//! #[actor]
//! impl MyActor {
//!     #[message]
//!     fn msg() -> Result<i32, Err> {}
//! }
//!
//! // Generates
//! impl MyActorRef {
//!     /// Sends the messages, waits for processing, and returns a response.
//!     async fn msg() -> Result<i32, SendError>;
//!     /// Sends the message after a delay.
//!     fn msg_after(delay: Duration) -> JoinHandle<Result<Result<i32, Err>, SendError>>;
//!     /// Sends the message asynchronously, not waiting for a response.
//!     fn msg_async() -> Result<(), SendError>;
//!     /// Sends the message asynchronously after a delay.
//!     fn msg_async_after(delay: Duration) -> JoinHandle<Result<(), SendError>>;
//! }
//! ```
//! **The \_after and \_async variants are only generated if the method does not have any lifetimes.**
//!
//! In other words, all parameters must be owned or `&'static` for the async variant to be generated,
//! otherwise the actor might reference deallocated memory causing UB.

#![warn(missing_docs)]
#![warn(clippy::all)]
#![warn(rust_2018_idioms)]
#![warn(missing_debug_implementations)]
#![deny(unused_must_use)]

mod actor;
mod actor_ref;
mod err;
#[doc(hidden)]
pub mod internal;
mod reason;
mod spawn;

pub use simpl_actor_macros::{actor, Actor};

pub use actor::*;
pub use actor_ref::*;
pub use err::*;
pub use reason::*;
pub use spawn::*;
