use std::{
    any::Any,
    fmt,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};

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

impl<E> std::error::Error for ActorError<E> {}

/// A shared error that occurs when an actor panics or returns an error from a hook in the [Actor](crate::Actor) trait.
#[derive(Clone)]
#[allow(missing_debug_implementations)]
pub struct PanicErr(Arc<Mutex<Box<dyn Any + Send>>>);

impl PanicErr {
    /// Creates a new PanicErr from a generic error.
    pub fn new<E>(err: E) -> Self
    where
        E: Send + 'static,
    {
        PanicErr(Arc::new(Mutex::new(Box::new(err))))
    }

    /// Creates a new PanicErr from a generic boxed error.
    pub fn new_boxed(err: Box<dyn Any + Send>) -> Self {
        PanicErr(Arc::new(Mutex::new(err)))
    }

    /// Returns some reference to the inner value if it is of type `T`, or None if it isn’t.
    pub fn with_downcast_ref<T, F, R>(
        &self,
        f: F,
    ) -> Result<R, PoisonError<MutexGuard<'_, Box<dyn Any + Send>>>>
    where
        T: 'static,
        F: FnOnce(Option<&T>) -> R,
    {
        let lock = self.0.lock()?;
        Ok(f(lock.downcast_ref()))
    }

    /// Returns a reference to the error as a `Box<dyn Any + Send>`.
    pub fn with_any<F, R>(
        &self,
        f: F,
    ) -> Result<R, PoisonError<MutexGuard<'_, Box<dyn Any + Send>>>>
    where
        F: FnOnce(&Box<dyn Any + Send>) -> R,
    {
        let lock = self.0.lock()?;
        Ok(f(&lock))
    }
}
