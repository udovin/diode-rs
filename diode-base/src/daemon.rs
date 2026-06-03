use std::any::{TypeId, type_name};
use std::collections::HashSet;
use std::marker::PhantomData;
use std::sync::Arc;

use async_trait::async_trait;
use diode::{
    AddServiceExt as _, App, AppBuilder, AppContext, Dependencies, Plugin, Service,
    ServiceDependencyExt as _, StdError,
};
use tokio::task::JoinSet;

/// Cooperative cancellation token used to signal daemons to shut down.
pub use tokio_util::sync::CancellationToken;

use crate::defer;

#[derive(Default)]
struct DaemonRegistry {
    daemons: Vec<Arc<dyn DynDaemon>>,
    types: HashSet<TypeId>,
}

impl DaemonRegistry {
    pub fn add_daemon<T>(&mut self, daemon: Arc<T>)
    where
        T: Daemon + 'static,
    {
        if !self.types.insert(TypeId::of::<T>()) {
            panic!("Daemon {} already added", type_name::<T>());
        }
        self.daemons.push(daemon);
    }

    pub fn has_daemon<T>(&self) -> bool
    where
        T: Daemon + 'static,
    {
        self.types.contains(&TypeId::of::<T>())
    }

    pub async fn run_daemons(
        &self,
        app: Arc<App>,
        shutdown: CancellationToken,
    ) -> Result<(), StdError> {
        let span = tracing::info_span!("daemons");
        let mut futures = JoinSet::new();
        tracing::info!(parent: &span, "Daemons starting");
        for daemon in self.daemons.iter() {
            let shutdown = shutdown.child_token();
            let app = app.clone();
            let daemon = daemon.clone();
            futures.spawn(async move { daemon.run(&app, shutdown).await });
        }
        tracing::info!(parent: &span, "Daemons running");
        defer! {
            tracing::info!(parent: &span, "Daemons stopped");
        };
        let first_result = futures.join_next().await;
        shutdown.cancel();
        if let Some(result) = first_result {
            result.map_err(Box::new)??;
            while let Some(result) = futures.join_next().await {
                result.map_err(Box::new)??;
            }
        }
        Ok(())
    }
}

/// A long-running background task managed by the application.
///
/// Daemons are started together by [`RunDaemonsExt::run_daemons`] and run until
/// the shared [`CancellationToken`] is cancelled (for example on shutdown) or
/// until any one of them returns, at which point the rest are signalled to stop.
/// Each daemon is given its own child cancellation token.
///
/// The default [`run`](Daemon::run) implementation does nothing and just waits
/// for cancellation, which is handy for a daemon that only needs to keep a
/// component alive.
///
/// Register a daemon with [`AddDaemonExt::add_daemon`] (a concrete instance) or
/// [`AddDaemonServiceExt::add_daemon_service`] (resolved from a [`Service`]).
pub trait Daemon: Send + Sync {
    /// Runs the daemon until `shutdown` is cancelled.
    ///
    /// Implementations should return promptly once the token is cancelled.
    /// Returning `Err` causes [`RunDaemonsExt::run_daemons`] to stop the other
    /// daemons and surface the error.
    fn run(
        &self,
        app: &App,
        shutdown: CancellationToken,
    ) -> impl Future<Output = Result<(), StdError>> + Send {
        let _ = app;
        async move {
            shutdown.cancelled_owned().await;
            Ok(())
        }
    }
}

#[async_trait]
trait DynDaemon: Send + Sync {
    async fn run(&self, app: &App, shutdown: CancellationToken) -> Result<(), StdError>;
}

#[async_trait]
impl<T> DynDaemon for T
where
    T: Daemon,
{
    async fn run(&self, app: &App, shutdown: CancellationToken) -> Result<(), StdError> {
        self.run(app, shutdown).await
    }
}

/// Runs every registered [`Daemon`] until shutdown.
pub trait RunDaemonsExt {
    /// Runs all registered daemons concurrently.
    ///
    /// Returns once the `shutdown` token is cancelled or the first daemon
    /// returns; in either case the remaining daemons are signalled to stop and
    /// awaited. Returns immediately if no daemons were registered.
    ///
    /// # Errors
    ///
    /// Returns the first error reported by a daemon, or the join error if a
    /// daemon task panics.
    fn run_daemons(
        self,
        shutdown: CancellationToken,
    ) -> impl Future<Output = Result<(), StdError>> + Send;
}

impl RunDaemonsExt for App {
    async fn run_daemons(self, shutdown: CancellationToken) -> Result<(), StdError> {
        Arc::new(self).run_daemons(shutdown).await
    }
}

impl RunDaemonsExt for Arc<App> {
    async fn run_daemons(self, shutdown: CancellationToken) -> Result<(), StdError> {
        match self.get_component_ref::<DaemonRegistry>() {
            Some(v) => v.run_daemons(self.clone(), shutdown).await,
            None => Ok(()),
        }
    }
}

/// Registers concrete [`Daemon`] instances on the application.
///
/// This extension lives on [`AppContext`], so daemons can be registered both
/// while configuring the builder and from within a plugin's `build`. A daemon is
/// identified by its type.
pub trait AddDaemonExt {
    /// Registers `daemon` to be run by [`RunDaemonsExt::run_daemons`].
    ///
    /// # Panics
    ///
    /// Panics if a daemon of type `T` is already registered. Guard with
    /// [`has_daemon`](AddDaemonExt::has_daemon) when the same type may be
    /// registered more than once.
    fn add_daemon<T>(&self, daemon: impl Into<Arc<T>>)
    where
        T: Daemon + 'static;

    /// Returns whether a daemon of type `T` is registered.
    fn has_daemon<T>(&self) -> bool
    where
        T: Daemon + 'static;
}

impl AddDaemonExt for AppContext {
    fn add_daemon<T>(&self, daemon: impl Into<Arc<T>>)
    where
        T: Daemon + 'static,
    {
        if !self.has_component::<DaemonRegistry>() {
            self.add_component(DaemonRegistry::default());
        }
        self.get_component_mut::<DaemonRegistry>()
            .unwrap()
            .add_daemon(daemon.into());
    }

    fn has_daemon<T>(&self) -> bool
    where
        T: Daemon + 'static,
    {
        self.get_component_ref::<DaemonRegistry>()
            .is_some_and(|registry| registry.has_daemon::<T>())
    }
}

struct DaemonServiceProvider<T>(PhantomData<T>);

impl<T> Plugin for DaemonServiceProvider<T>
where
    T: Service<Handle = Arc<T>> + Daemon + 'static,
{
    async fn build(&self, ctx: &AppContext) -> Result<(), StdError> {
        let handle = ctx.get_component::<T::Handle>().unwrap();
        ctx.add_daemon::<T>(handle);
        Ok(())
    }

    fn dependencies(&self) -> Dependencies {
        T::dependencies().service::<T>()
    }
}

/// Registers daemons resolved from the dependency-injection container.
///
/// The daemon type `T` is a [`Service`]: it is built by the container (together
/// with its dependencies) and its handle is registered as a [`Daemon`]. The
/// service is added automatically if it is not already present.
pub trait AddDaemonServiceExt {
    /// Registers the [`Service`] `T` to be run as a daemon.
    ///
    /// # Panics
    ///
    /// Panics if `T` is already registered as a daemon service (its provider
    /// plugin would be added twice); guard with
    /// [`has_daemon_service`](AddDaemonServiceExt::has_daemon_service) when this
    /// can happen. Building the [`App`] additionally panics if `T` is registered
    /// both as a daemon service and as an instance via
    /// [`AddDaemonExt::add_daemon`].
    fn add_daemon_service<T>(&mut self) -> &mut Self
    where
        T: Service<Handle = Arc<T>> + Daemon + 'static;

    /// Returns whether `T` is registered as a daemon service.
    fn has_daemon_service<T>(&self) -> bool
    where
        T: Service<Handle = Arc<T>> + Daemon + 'static;
}

impl AddDaemonServiceExt for AppBuilder {
    fn add_daemon_service<T>(&mut self) -> &mut Self
    where
        T: Service<Handle = Arc<T>> + Daemon + 'static,
    {
        if !self.has_service::<T>() {
            self.add_service::<T>();
        }
        self.add_plugin(DaemonServiceProvider::<T>(PhantomData));
        self
    }

    fn has_daemon_service<T>(&self) -> bool
    where
        T: Service<Handle = Arc<T>> + Daemon + 'static,
    {
        self.has_plugin::<DaemonServiceProvider<T>>()
    }
}
