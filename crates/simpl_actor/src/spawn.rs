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
}
