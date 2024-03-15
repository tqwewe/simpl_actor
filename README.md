# simpl_actor

`simpl_actor` is a Rust library for simplifying actor-based concurrency in Rust applications. It is built on top of Tokio, utilizing async/await and tokio mpsc channels for efficient and intuitive actor system implementation.

## Features

- **Simple Actor Definition**: Easily define actors with Rust structs and an attribute macro.
- **Automatic Message Handling**: Implement message handling with minimal code, leveraging Rust's type system and async capabilities.
- **Asynchronous and Synchronous Messaging**: Support for both asynchronous and synchronous message processing, allowing for flexible actor interaction patterns.
- **Lifecycle Management**: Lifecycle hooks for initializing, restarting, and stopping actors, providing control over the actor lifecycle.

## Quick Start

Define an actor:

```rust
use simpl_actor::{actor, Actor, Spawn};

#[derive(Actor)]
pub struct CounterActor {
    count: i64,
}

#[actor]
impl CounterActor {
    pub fn new() -> Self {
        CounterActor { count: 0 }
    }

    #[message]
    pub fn inc(&mut self, amount: i64) { ... }

    #[message]
    pub fn dec(&mut self, amount: i64) { ... }

    #[message]
    pub fn count(&self) -> i64 { ... }
}
```

Interact with the actor:

```rust
let counter = CounterActor::new();
let actor = counter.spawn();

actor.inc(2).await?;
actor.dec(1).await?;
let count = actor.count().await?;
```

## Messaging Variants

When you define a message in `simpl_actor`, six variants of the message handling function are automatically generated to offer flexibility in how messages are sent and processed:

```rust
#[actor]
impl MyActor {
    #[message]
    fn msg() -> i32 {}
    }

// Generates
impl MyActorRef {
    /// Sends the messages, waits for processing, and returns a response.
    async fn msg() -> Result<i32, ActorError> {}
    /// Sends the message with a timeout for adding to the mailbox if the mailbox is full.
    async fn msg_timeout(timeout: Duration) -> Result<i32, ActorError> {}
    /// Attempts to send the message immediately without waiting for mailbox capacity.
    async fn try_msg() -> Result<i32, ActorError> {}
    /// Sends the message asynchronously, not waiting for a response.
    async fn msg_async() -> Result<(), ActorError> {}
    /// Sends the message asyncronously with a timeout for mailbox capacity.
    async fn msg_async_timeout(timeout: Duration) -> Result<(), ActorError> {}
    /// Attempts to immediately send the message asyncronously without waiting for a response or mailbox capacity.
    fn try_msg_async() -> Result<(), ActorError> {}
}
```

**Async variants (`_async`, `_async_timeout`, and `try_async`) are only generated if the method does not have any lifetimes.**
In other words, all parameters must be owned or `&'static` for async variants to be generated, otherwise the actor might reference deallocated memory causing UB.

These variants provide a range of options for how and when messages are processed by the actor, from synchronous waiting to non-blocking attempts, with or without timeouts.

## Contributing

Contributions are welcome! Feel free to submit pull requests, create issues, or suggest improvements.

Sure, here's the revised license section for your README.md:

---

## License

`simpl_actor` is dual-licensed under either:

- MIT License ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)
- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)

at your option.

This means you can choose the license that best suits your project's needs.
