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
//! When you define a message in `simpl_actor`, six variants of the message handling function are
//! automatically generated to offer flexibility in how messages are sent and processed:
//!
//! ```
//! #[actor]
//! impl MyActor {
//!     #[message]
//!     fn msg() -> i32 {}
//! }
//!
//! // Generates
//! impl MyActorRef {
//!     /// Sends the messages, waits for processing, and returns a response.
//!     async fn msg() -> Result<i32, ActorError> {}
//!     /// Sends the message with a timeout for adding to the mailbox if the mailbox is full.
//!     async fn msg_timeout(timeout: Duration) -> Result<i32, ActorError> {}
//!     /// Attempts to send the message immediately without waiting for mailbox capacity.
//!     async fn try_msg() -> Result<i32, ActorError> {}
//!     /// Sends the message asynchronously, not waiting for a response.
//!     async fn msg_async() -> Result<(), ActorError> {}
//!     /// Sends the message asyncronously with a timeout for mailbox capacity.
//!     async fn msg_async_timeout(timeout: Duration) -> Result<(), ActorError> {}
//!     /// Attempts to immediately send the message asyncronously without waiting for a response or mailbox capacity.
//!     fn try_msg_async() -> Result<(), ActorError> {}
//! }
//! ```
//! **Async variants (`_async`, `_async_timeout`, and `try_async`) are only generated if the method does not have any lifetimes.**
//!
//! In other words, all parameters must be owned or `&'static` for async variants to be generated, otherwise the actor might reference deallocated memory causing UB.
//!
//! These variants provide a range of options for how and when messages are processed by the actor, from synchronous waiting to non-blocking attempts, with or without timeouts.

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
mod stop_error_hook;

pub use simpl_actor_macros::{actor, Actor};

pub use actor::*;
pub use actor_ref::*;
pub use err::*;
pub use reason::*;
pub use spawn::*;
pub use stop_error_hook::*;
