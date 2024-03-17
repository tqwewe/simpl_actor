use std::{
    any::Any,
    fmt,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};

/// Error that can occur when sending a message to an actor.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SendError<E = ()> {
    /// The actor isn't running.
    ActorNotRunning(E),
    /// The actor panicked or was stopped.
    ActorStopped,
}

impl<E> fmt::Debug for SendError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SendError::ActorNotRunning(_) => {
                write!(f, "ActorNotRunning")
            }
            SendError::ActorStopped => write!(f, "ActorStopped"),
        }
    }
}

impl<E> fmt::Display for SendError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SendError::ActorNotRunning(_) => write!(f, "actor not running"),
            SendError::ActorStopped => write!(f, "actor stopped"),
        }
    }
}

impl<E> std::error::Error for SendError<E> {}

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

    /// Calls the passed closure `f` with an option containing the boxed any type downcasted into a `Cow<'static, str>`,
    /// or `None` if it's not a string type.
    pub fn with_str<F, R>(
        &self,
        f: F,
    ) -> Result<Option<R>, PoisonError<MutexGuard<'_, Box<dyn Any + Send>>>>
    where
        F: FnOnce(&str) -> R,
    {
        self.with(|any| {
            any.downcast_ref::<&'static str>()
                .copied()
                .or_else(|| any.downcast_ref::<String>().map(String::as_str))
                .map(|s| f(s))
        })
    }

    /// Calls the passed closure `f` with the inner type downcasted into `T`, otherwise returns `None`.
    pub fn with_downcast_ref<T, F, R>(
        &self,
        f: F,
    ) -> Result<Option<R>, PoisonError<MutexGuard<'_, Box<dyn Any + Send>>>>
    where
        T: 'static,
        F: FnOnce(&T) -> R,
    {
        let lock = self.0.lock()?;
        Ok(lock.downcast_ref().map(f))
    }

    /// Returns a reference to the error as a `Box<dyn Any + Send>`.
    pub fn with<F, R>(&self, f: F) -> Result<R, PoisonError<MutexGuard<'_, Box<dyn Any + Send>>>>
    where
        F: FnOnce(&Box<dyn Any + Send>) -> R,
    {
        let lock = self.0.lock()?;
        Ok(f(&lock))
    }
}
