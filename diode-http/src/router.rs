use std::any::{TypeId, type_name};
use std::collections::HashSet;
use std::marker::PhantomData;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use diode::{
    AddServiceExt as _, App, AppBuilder, AppContext, Dependencies, Plugin, Service,
    ServiceDependencyExt as _, StdError,
};
use diode_base::{AddDaemonExt as _, CancellationToken, Config, Daemon, config_section, defer};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;

use crate::tracing::TracingLayer;

pub trait RouterBuilder: Send + Sync {
    fn build_router(self: Arc<Self>, app: &App) -> Router;
}

#[derive(Default)]
struct RouterRegistry {
    routers: Vec<Arc<dyn RouterBuilder>>,
    types: HashSet<TypeId>,
}

impl RouterRegistry {
    fn add_router<T: RouterBuilder + 'static>(&mut self, router: Arc<T>) {
        if !self.types.insert(TypeId::of::<T>()) {
            panic!("Router {} already added", type_name::<T>());
        }
        self.routers.push(router);
    }

    fn has_router<T: RouterBuilder + 'static>(&self) -> bool {
        self.types.contains(&TypeId::of::<T>())
    }

    fn build_router(&self, app: &App) -> Router {
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
    async fn build(&self, ctx: &AppContext) -> Result<(), StdError> {
        if !ctx.has_component::<RouterRegistry>() {
            ctx.add_component(RouterRegistry::default());
        }
        let config = ctx
            .get_component_ref::<Config>()
            .ok_or_else(|| "Config component is missing".to_string())?
            .get::<HttpServerConfig>("http_server")?;
        ctx.add_daemon(ServerDaemon { addr: config.addr });
        Ok(())
    }
}

struct RouterProvider<T>(PhantomData<T>);

impl<T> Plugin for RouterProvider<T>
where
    T: Service<Handle = Arc<T>> + RouterBuilder + 'static,
{
    async fn build(&self, ctx: &AppContext) -> Result<(), StdError> {
        let component = ctx.get_component::<T::Handle>().unwrap();
        ctx.get_component_mut::<RouterRegistry>()
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
    fn add_router<T>(&self, router: impl Into<Arc<T>>)
    where
        T: RouterBuilder + 'static;

    fn has_router<T>(&self) -> bool
    where
        T: RouterBuilder + 'static;
}

impl AddRouterExt for AppContext {
    fn add_router<T>(&self, router: impl Into<Arc<T>>)
    where
        T: RouterBuilder + 'static,
    {
        if !self.has_component::<RouterRegistry>() {
            self.add_component(RouterRegistry::default());
        }
        self.get_component_mut::<RouterRegistry>()
            .unwrap()
            .add_router(router.into());
    }

    fn has_router<T>(&self) -> bool
    where
        T: RouterBuilder + 'static,
    {
        self.get_component_ref::<RouterRegistry>()
            .is_some_and(|registry| registry.has_router::<T>())
    }
}

pub trait AddRouterServiceExt {
    fn add_router_service<T>(&mut self) -> &mut Self
    where
        T: Service<Handle = Arc<T>> + RouterBuilder + 'static;

    fn has_router_service<T>(&self) -> bool
    where
        T: Service<Handle = Arc<T>> + RouterBuilder + 'static;
}

impl AddRouterServiceExt for AppBuilder {
    fn add_router_service<T>(&mut self) -> &mut Self
    where
        T: Service<Handle = Arc<T>> + RouterBuilder + 'static,
    {
        if !self.has_service::<T>() {
            self.add_service::<T>();
        }
        self.add_plugin(RouterProvider::<T>(PhantomData));
        self
    }

    fn has_router_service<T>(&self) -> bool
    where
        T: Service<Handle = Arc<T>> + RouterBuilder + 'static,
    {
        self.has_plugin::<RouterProvider<T>>()
    }
}
