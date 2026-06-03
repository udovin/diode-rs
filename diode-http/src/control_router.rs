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
use crate::{HealthCheckRegistry, HealthClient, RouterBuilder};

#[derive(Default)]
struct ControlRouterRegistry {
    routers: Vec<Arc<dyn RouterBuilder>>,
    types: HashSet<TypeId>,
}

impl ControlRouterRegistry {
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

struct ControlServerDaemon {
    addr: SocketAddr,
}

impl Daemon for ControlServerDaemon {
    async fn run(&self, app: &App, shutdown: CancellationToken) -> Result<(), StdError> {
        let span = tracing::info_span!("control_server", addr = ?self.addr);
        let router = app
            .get_component_ref::<ControlRouterRegistry>()
            .unwrap()
            .build_router(app)
            .layer(TracingLayer);
        tracing::info!(parent: &span, "Control server starting");
        defer! {
            tracing::info!(parent: &span, "Control server stopped")
        };
        let listener = TcpListener::bind(self.addr).await.map_err(Box::new)?;
        tracing::info!(parent: &span, "Control server started");
        axum::serve(listener, router)
            .with_graceful_shutdown(shutdown.cancelled_owned())
            .await
            .map_err(Box::new)?;
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
#[config_section("control_server")]
pub struct ControlServerConfig {
    pub addr: SocketAddr,
}

pub struct ControlServerPlugin;

impl Plugin for ControlServerPlugin {
    async fn build(&self, ctx: &AppContext) -> Result<(), StdError> {
        if !ctx.has_component::<ControlRouterRegistry>() {
            ctx.add_component(ControlRouterRegistry::default());
        }
        if !ctx.has_component::<HealthCheckRegistry>() {
            ctx.add_component(HealthCheckRegistry::default());
        }
        let config = ctx
            .get_component_ref::<Config>()
            .ok_or_else(|| "Config component is missing".to_string())?
            .get::<ControlServerConfig>("control_server")?;
        ctx.add_component(HealthClient::new(format!("http://{}/health", config.addr)));
        ctx.add_daemon(ControlServerDaemon { addr: config.addr });
        Ok(())
    }
}

struct ControlRouterProvider<T>(PhantomData<T>);

impl<T> Plugin for ControlRouterProvider<T>
where
    T: Service<Handle = Arc<T>> + RouterBuilder + 'static,
{
    async fn build(&self, ctx: &AppContext) -> Result<(), StdError> {
        let component = ctx.get_component::<T::Handle>().unwrap();
        ctx.get_component_mut::<ControlRouterRegistry>()
            .unwrap()
            .add_router(component);
        Ok(())
    }

    fn dependencies(&self) -> Dependencies {
        T::dependencies()
            .service::<T>()
            .plugin::<ControlServerPlugin>()
    }
}

pub trait AddControlRouterExt {
    fn add_control_router<T>(&self, router: impl Into<Arc<T>>)
    where
        T: RouterBuilder + 'static;

    fn has_control_router<T>(&self) -> bool
    where
        T: RouterBuilder + 'static;
}

impl AddControlRouterExt for AppContext {
    fn add_control_router<T>(&self, router: impl Into<Arc<T>>)
    where
        T: RouterBuilder + 'static,
    {
        if !self.has_component::<ControlRouterRegistry>() {
            self.add_component(ControlRouterRegistry::default());
        }
        self.get_component_mut::<ControlRouterRegistry>()
            .unwrap()
            .add_router(router.into());
    }

    fn has_control_router<T>(&self) -> bool
    where
        T: RouterBuilder + 'static,
    {
        self.get_component_ref::<ControlRouterRegistry>()
            .is_some_and(|registry| registry.has_router::<T>())
    }
}

pub trait AddControlRouterServiceExt {
    /// Registers a router resolved from a DI [`Service`] on the control server.
    fn add_control_router_service<T>(&mut self) -> &mut Self
    where
        T: Service<Handle = Arc<T>> + RouterBuilder + 'static;

    fn has_control_router_service<T>(&self) -> bool
    where
        T: Service<Handle = Arc<T>> + RouterBuilder + 'static;
}

impl AddControlRouterServiceExt for AppBuilder {
    fn add_control_router_service<T>(&mut self) -> &mut Self
    where
        T: Service<Handle = Arc<T>> + RouterBuilder + 'static,
    {
        if !self.has_service::<T>() {
            self.add_service::<T>();
        }
        self.add_plugin(ControlRouterProvider::<T>(PhantomData));
        self
    }

    fn has_control_router_service<T>(&self) -> bool
    where
        T: Service<Handle = Arc<T>> + RouterBuilder + 'static,
    {
        self.has_plugin::<ControlRouterProvider<T>>()
    }
}
