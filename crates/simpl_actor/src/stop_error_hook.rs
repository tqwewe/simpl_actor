use std::{any::Any, future::Future, mem, pin::Pin};

use tokio::sync::RwLock;
use tracing::error;

type OnStopErrorHookFn =
    Box<dyn Fn(u64, Box<dyn Any + Send>) -> Pin<Box<dyn Future<Output = ()> + Send>> + Sync + Send>;

static ON_STOP_ERROR_HOOK: RwLock<StopErrorHook> = RwLock::const_new(StopErrorHook::Default);

#[derive(Default)]
pub(crate) enum StopErrorHook {
    #[default]
    Default,
    Custom(OnStopErrorHookFn),
}

impl StopErrorHook {
    pub(crate) async fn call_hook(id: u64, err: Box<dyn Any + Send>) {
        let hook = ON_STOP_ERROR_HOOK.read().await;
        match *hook {
            StopErrorHook::Default => default_on_stop_error_hook(id, err),
            StopErrorHook::Custom(ref hook) => {
                hook(id, err).await;
            }
        }
    }
}

/// Registers a custom hook called when any actor's [Actor::on_stop](crate::actor::Actor::on_stop) hook returns an error.
///
/// The default hook prints a message using the [tracing::error!](https://docs.rs/tracing/latest/tracing/macro.error.html) macro.
/// To see errors printed by the default hook, a [tracing subscriber](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/) must be set.
///
/// The on_stop error hook is a global resource.
///
/// # Example
///
/// ```
/// simpl_actor::set_on_stop_error_hook(Box::new(|_, _| {
///     Box::pin(async move {
///         println!("actor {id} errored");
///     })
/// }));
/// ```
pub async fn set_on_stop_error_hook(hook: OnStopErrorHookFn) {
    let new = StopErrorHook::Custom(hook);
    let mut hook = ON_STOP_ERROR_HOOK.write().await;
    let old = mem::replace(&mut *hook, new);
    mem::drop(hook);
    // Only drop the old hook after releasing the lock to avoid deadlocking
    // if its destructor panics.
    mem::drop(old);
}

fn default_on_stop_error_hook(id: u64, _err: Box<dyn Any + Send>) {
    error!("actor #{id} on_stop hook errored");
}
