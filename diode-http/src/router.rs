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

/// Builds the [`axum::Router`] contributed by a type to an HTTP server.
///
/// Implement this for any value that exposes routes. The builder is handed the
/// fully built [`App`], so it can resolve other components (services,
/// registries, configuration) while assembling its routes.
///
/// Register an implementor with [`AddRouterExt::add_router`] (a concrete
/// instance) or [`AddRouterServiceExt::add_router_service`] (resolved from a
/// [`Service`]). The server merges every registered router into one.
pub trait RouterBuilder: Send + Sync {
    /// Builds this type's routes into a [`Router`].
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

/// Configuration for the public HTTP server, read from the `http_server`
/// config section.
#[derive(Serialize, Deserialize)]
#[config_section("http_server")]
pub struct HttpServerConfig {
    /// Socket address the server binds and listens on.
    pub addr: SocketAddr,
}

/// Plugin that runs the public HTTP server.
///
/// Add it to the [`AppBuilder`] to serve every router registered through
/// [`AddRouterExt`] / [`AddRouterServiceExt`]. The server binds the address from
/// [`HttpServerConfig`] (config section `http_server`) and runs as a [`Daemon`],
/// shutting down gracefully when its cancellation token fires.
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

/// Registers concrete [`RouterBuilder`] instances on the public HTTP server.
///
/// This extension lives on [`AppContext`], so routers can be registered both
/// while configuring the [`AppBuilder`] and from within a plugin's `build`. A
/// router is identified by its type: at most one instance of a given type may be
/// registered.
///
/// Registration does not require [`HttpServerPlugin`]; if the plugin is never
/// added the router is simply never served.
pub trait AddRouterExt {
    /// Registers `router` on the public HTTP server.
    ///
    /// The router is stored under its concrete type `T`. To expose several
    /// distinct sets of routes use distinct types; a single type that needs many
    /// routes should build them all in its [`RouterBuilder`] implementation.
    ///
    /// # Panics
    ///
    /// Panics if a router of type `T` is already registered on this server.
    /// Guard with [`has_router`](AddRouterExt::has_router) when the same type may
    /// be registered more than once (for example from several bundles).
    fn add_router<T>(&self, router: impl Into<Arc<T>>)
    where
        T: RouterBuilder + 'static;

    /// Returns whether a router of type `T` is registered on the public server.
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

/// Registers routers resolved from the dependency-injection container on the
/// public HTTP server.
///
/// Unlike [`AddRouterExt`], the router type `T` is a [`Service`]: it is built by
/// the container (together with its dependencies) and its handle is then
/// registered as a [`RouterBuilder`]. The service is added automatically if it
/// is not already present.
pub trait AddRouterServiceExt {
    /// Registers the [`Service`] `T` and serves its routes on the public server.
    ///
    /// # Panics
    ///
    /// Panics if `T` is already registered as a router service (its provider
    /// plugin would be added twice); guard with
    /// [`has_router_service`](AddRouterServiceExt::has_router_service) when this
    /// can happen. Building the [`App`] additionally panics if `T` is registered
    /// both as a service router and as an instance via
    /// [`AddRouterExt::add_router`], since a type may back at most one router.
    fn add_router_service<T>(&mut self) -> &mut Self
    where
        T: Service<Handle = Arc<T>> + RouterBuilder + 'static;

    /// Returns whether `T` is registered as a router service on the public
    /// server.
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
