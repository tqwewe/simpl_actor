use std::fmt;

use crate::err::PanicErr;

/// Reason for an actor being stopped.
#[derive(Clone)]
pub enum ActorStopReason {
    /// Actor stopped normally.
    Normal,
    /// Actor was killed.
    Killed,
    /// Actor panicked.
    Panicked(PanicErr),
    /// Link died.
    LinkDied {
        /// Actor ID.
        id: u64,
        /// Actor died reason.
        reason: Box<ActorStopReason>,
    },
}

impl fmt::Debug for ActorStopReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ActorStopReason::Normal => write!(f, "Normal"),
            ActorStopReason::Killed => write!(f, "Killed"),
            ActorStopReason::Panicked(_) => write!(f, "Panicked"),
            ActorStopReason::LinkDied { id, reason } => f
                .debug_struct("LinkDied")
                .field("id", id)
                .field("reason", &reason)
                .finish(),
        }
    }
}

impl fmt::Display for ActorStopReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ActorStopReason::Normal => write!(f, "actor stopped normally"),
            ActorStopReason::Killed => write!(f, "actor was killed"),
            ActorStopReason::Panicked(_) => write!(f, "actor panicked"),
            ActorStopReason::LinkDied { id, reason } => {
                write!(f, "link {id} died with reason: {}", reason.as_ref())
            }
        }
    }
}
