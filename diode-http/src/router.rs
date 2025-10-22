use std::marker::PhantomData;
use std::net::SocketAddr;
use std::sync::Arc;

use async_trait::async_trait;
use axum::Router;
use diode::{
    AddServiceExt as _, App, AppBuilder, Dependencies, Plugin, Service, ServiceDependencyExt as _,
    StdError,
};
use diode_base::{AddDaemonExt as _, CancellationToken, Config, Daemon, config_section, defer};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;

use crate::tracing::TracingLayer;

#[derive(Default)]
struct RouterRegistry {
    routers: Vec<Arc<dyn DynRouterBuilder>>,
}

impl RouterRegistry {
    pub fn add_router<T: RouterBuilder + 'static>(&mut self, router: Arc<T>) {
        self.routers.push(router);
    }

    pub fn build_router(&self, app: &App) -> Router {
        self.routers.iter().fold(Router::new(), |acc, v| {
            acc.merge(v.clone().build_router(app))
        })
    }
}

struct ServerDaemon {
    addr: SocketAddr,
}

impl Daemon for ServerDaemon {
    async fn run(&self, app: &App, shutdown: CancellationToken) -> Result<(), StdError> {
        let span = tracing::info_span!("http_server", addr = ?self.addr);
        let router = app
            .get_component_ref::<RouterRegistry>()
            .unwrap()
            .build_router(app)
            .layer(TracingLayer);
        tracing::info!(parent: &span, "Server starting");
        defer! {
            tracing::info!(parent: &span, "Server stopped")
        };
        let listener = TcpListener::bind(self.addr).await.map_err(Box::new)?;
        tracing::info!(parent: &span, "Server started");
        axum::serve(listener, router)
            .with_graceful_shutdown(shutdown.cancelled_owned())
            .await
            .map_err(Box::new)?;
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
#[config_section("http_server")]
pub struct HttpServerConfig {
    pub addr: SocketAddr,
}

pub struct HttpServerPlugin;

impl Plugin for HttpServerPlugin {
    async fn build(&self, app: &mut AppBuilder) -> Result<(), StdError> {
        app.add_component(RouterRegistry::default());
        let config = app
            .get_component_ref::<Config>()
            .ok_or_else(|| "Config component is missing".to_string())?
            .get::<HttpServerConfig>("http_server")?;
        app.add_daemon(ServerDaemon { addr: config.addr });
        Ok(())
    }
}

pub trait RouterBuilder: Send + Sync {
    fn build_router(self: Arc<Self>, app: &App) -> Router;
}

#[async_trait]
pub(crate) trait DynRouterBuilder: Send + Sync {
    fn build_router(self: Arc<Self>, app: &App) -> Router;
}

impl<T> DynRouterBuilder for T
where
    T: RouterBuilder,
{
    fn build_router(self: Arc<Self>, app: &App) -> Router {
        self.build_router(app)
    }
}

struct RouterProvider<T>(PhantomData<T>);

impl<T> Plugin for RouterProvider<T>
where
    T: Service<Handle = Arc<T>> + RouterBuilder + 'static,
{
    async fn build(&self, app: &mut AppBuilder) -> Result<(), StdError> {
        let component = app.get_component::<T::Handle>().unwrap();
        app.get_component_mut::<RouterRegistry>()
            .unwrap()
            .add_router(component);
        Ok(())
    }

    fn dependencies(&self) -> Dependencies {
        T::dependencies()
            .service::<T>()
            .plugin::<HttpServerPlugin>()
    }
}

pub trait AddRouterExt {
    fn add_router<T>(&mut self) -> &mut Self
    where
        T: Service<Handle = Arc<T>> + RouterBuilder + 'static;

    fn has_router<T>(&self) -> bool
    where
        T: Service<Handle = Arc<T>> + RouterBuilder + 'static;
}

impl AddRouterExt for AppBuilder {
    fn add_router<T>(&mut self) -> &mut Self
    where
        T: Service<Handle = Arc<T>> + RouterBuilder + 'static,
    {
        if !self.has_service::<T>() {
            self.add_service::<T>();
        }
        self.add_plugin(RouterProvider::<T>(PhantomData));
        self
    }

    fn has_router<T>(&self) -> bool
    where
        T: Service<Handle = Arc<T>> + RouterBuilder + 'static,
    {
        self.has_plugin::<RouterProvider<T>>()
    }
}
