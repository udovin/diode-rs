use std::marker::PhantomData;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use diode::{
    AddServiceExt as _, App, AppBuilder, Dependencies, Plugin, Service, ServiceDependencyExt as _,
    StdError,
};
use diode_base::{AddDaemonExt as _, CancellationToken, Config, Daemon, config_section, defer};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;

use crate::tracing::TracingLayer;
use crate::{DynRouterBuilder, HealthCheckRegistry, HealthClient, RouterBuilder};

#[derive(Default)]
struct ServiceRouterRegistry {
    routers: Vec<Arc<dyn DynRouterBuilder>>,
}

impl ServiceRouterRegistry {
    pub fn add_router<T: RouterBuilder + 'static>(&mut self, router: Arc<T>) {
        self.routers.push(router);
    }

    pub fn build_router(&self, app: &App) -> Router {
        self.routers.iter().fold(Router::new(), |acc, v| {
            acc.merge(v.clone().build_router(app))
        })
    }
}

struct ServiceServerDaemon {
    addr: SocketAddr,
}

impl Daemon for ServiceServerDaemon {
    async fn run(&self, app: &App, shutdown: CancellationToken) -> Result<(), StdError> {
        let span = tracing::info_span!("service_http_server", addr = ?self.addr);
        let router = app
            .get_component_ref::<ServiceRouterRegistry>()
            .unwrap()
            .build_router(app)
            .layer(TracingLayer);
        tracing::info!(parent: &span, "Service server starting");
        defer! {
            tracing::info!(parent: &span, "Service server stopped")
        };
        let listener = TcpListener::bind(self.addr).await.map_err(Box::new)?;
        tracing::info!(parent: &span, "Service server started");
        axum::serve(listener, router)
            .with_graceful_shutdown(shutdown.cancelled_owned())
            .await
            .map_err(Box::new)?;
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
#[config_section("service_http_server")]
pub struct ServiceServerConfig {
    pub addr: SocketAddr,
}

pub struct ServiceServerPlugin;

impl Plugin for ServiceServerPlugin {
    async fn build(&self, app: &mut AppBuilder) -> Result<(), StdError> {
        app.add_component(ServiceRouterRegistry::default());
        app.add_component(HealthCheckRegistry::default());
        let config = app
            .get_component_ref::<Config>()
            .ok_or_else(|| "Config component is missing".to_string())?
            .get::<ServiceServerConfig>("service_http_server")?;
        app.add_component(HealthClient::new(format!("http://{}", config.addr)));
        app.add_daemon(ServiceServerDaemon { addr: config.addr });
        Ok(())
    }
}

struct ServiceRouterProvider<T>(PhantomData<T>)
where
    T: Service<Handle = Arc<T>> + RouterBuilder;

impl<T> Plugin for ServiceRouterProvider<T>
where
    T: Service<Handle = Arc<T>> + RouterBuilder + 'static,
{
    async fn build(&self, app: &mut AppBuilder) -> Result<(), StdError> {
        let component = app.get_component::<T::Handle>().unwrap();
        app.get_component_mut::<ServiceRouterRegistry>()
            .unwrap()
            .add_router(component);
        Ok(())
    }

    fn dependencies(&self) -> Dependencies {
        T::dependencies()
            .service::<T>()
            .plugin::<ServiceServerPlugin>()
    }
}

pub trait AddServiceRouterExt {
    fn add_service_router<T>(&mut self) -> &mut Self
    where
        T: Service<Handle = Arc<T>> + RouterBuilder + 'static;

    fn has_service_router<T>(&self) -> bool
    where
        T: Service<Handle = Arc<T>> + RouterBuilder + 'static;
}

impl AddServiceRouterExt for AppBuilder {
    fn add_service_router<T>(&mut self) -> &mut Self
    where
        T: Service<Handle = Arc<T>> + RouterBuilder + 'static,
    {
        if !self.has_service::<T>() {
            self.add_service::<T>();
        }
        self.add_plugin(ServiceRouterProvider::<T>(PhantomData));
        self
    }

    fn has_service_router<T>(&self) -> bool
    where
        T: Service<Handle = Arc<T>> + RouterBuilder + 'static,
    {
        self.has_plugin::<ServiceRouterProvider<T>>()
    }
}
