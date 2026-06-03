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

pub trait Daemon: Send + Sync {
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

pub trait RunDaemonsExt {
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

pub trait AddDaemonExt {
    fn add_daemon<T>(&self, daemon: impl Into<Arc<T>>)
    where
        T: Daemon + 'static;

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

pub trait AddDaemonServiceExt {
    fn add_daemon_service<T>(&mut self) -> &mut Self
    where
        T: Service<Handle = Arc<T>> + Daemon + 'static;

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
