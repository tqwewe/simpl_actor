mod actor;
mod derive_actor;

use actor::Actor;
use derive_actor::DeriveActor;
use proc_macro::TokenStream;
use quote::ToTokens;
use syn::parse_macro_input;

/// Attribute macro placed on `impl` blocks of actors to define messages.
///
/// Methods on the impl block are marked with `#[message]`, allowing them to be called on the actor.
///
/// **Infallible messages**
///
/// All messages (besides those with no return value) should return a `Result`.
/// If a message returns anything other than a result, it should be explicitly marked as infallible with `#[message(infallible)]`.
///
/// **Internals**
///
/// Under the hood, this macro generates the following (among other things):
///
/// - `pub struct MyActorRef`: a reference to this actor which implements [ActorRef](https://docs.rs/simpl_actor/latest/simpl_actor/trait.ActorRef.html) and has methods to interact with the actor based on the messages defined.
/// - `enum MyActorMsg`: a private message enum used in the mpsc channel.
/// - `impl Spawn for MyActor`: an implementation of [Spawn](https://docs.rs/simpl_actor/latest/simpl_actor/trait.Spawn.html), allowing the actor to be spawned.
///
/// The actor being implemented should implement [Actor](https://docs.rs/simpl_actor/latest/simpl_actor/trait.Actor.html) either with the derive macro, or manually.
///
/// *Note that generics are not supported in messages, instead dyn trait objects should be used.*
///
/// Internally, tokio a tokio mpsc unbounded channel is used for the mailbox.
/// This macro generates a message enum and boilerplate for sending messages to the actors mailbox, and receiving replies.
///
/// # Example
///
/// ```
/// #[actor]
/// impl MyActor {
///     pub fn new() -> Self { ... }
///
///     /// A message with no return value
///     #[message]
///     pub fn inc(&mut self, amount: i64) {
///         self.count += amount;
///     }
///
///     /// A message returning a value
///     #[message(infallible)]
///     pub fn count(&self) -> i64 {
///         self.count
///     }
///
///     /// A message that uses async
///     #[message]
///     pub async fn sleep(&self) {
///         tokio::time::sleep(Duration::from_millis(500)).await;
///     }
///
///     /// A message that sends a message to itself
///     #[message]
///     pub async fn inc_myself(&self) -> Result<(), SendError<i64>> {
///         self.actor_ref().inc_async(1).await
///     }
/// }
/// ```
#[proc_macro_attribute]
pub fn actor(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let actor = parse_macro_input!(item as Actor);
    let foo = actor.into_token_stream();
    std::fs::write("./code.rs", foo.to_string()).unwrap();
    TokenStream::from(foo)
}

/// Derive macro implementing the [Actor](https://docs.rs/simpl_actor/latest/simpl_actor/trait.Actor.html) trait with default behaviour.
///
/// # Example
///
/// ```
/// use simpl_actor::Actor;
///
/// #[derive(Actor)]
/// struct MyActor { }
/// ```
#[proc_macro_derive(Actor)]
pub fn derive_actor(input: TokenStream) -> TokenStream {
    let derive_actor = parse_macro_input!(input as DeriveActor);
    TokenStream::from(derive_actor.into_token_stream())
}
