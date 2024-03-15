/// Provides functionality to spawn actors and obtain references to them.
///
/// This trait defines how actors are spawned and allows for retrieving references to the actor.
/// Implementing this trait enables the creation and management of actor instances.
///
/// This trait is automatically implemented by the [actor!](macro@crate::actor) macro.
#[allow(async_fn_in_trait)]
pub trait Spawn {
    /// The type of reference to the actor, allowing for message sending and interaction.
    type Ref;

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
    /// If either actor dies, [Actor::on_link_died](crate::actor::Actor::on_link_died) will be called on the other actor.
    async fn spawn_link(self) -> Self::Ref;

    /// Spawns an actor with a unidirectional link between the current actor and the child.
    ///
    /// If the current actor dies, [Actor::on_link_died](crate::actor::Actor::on_link_died) will be called on the spawned one,
    /// however if the spawned actor dies, Actor::on_link_died will not be called.
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
