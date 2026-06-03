use async_trait::async_trait;
use axum::{Router, routing};
use diode::{
    AddServiceExt as _, App, AppBuilder, AppContext, Dependencies, Plugin, Service,
    ServiceDependencyExt as _, StdError,
};
use serde::{Deserialize, Serialize};
use std::{
    any::{TypeId, type_name},
    collections::HashSet,
    marker::PhantomData,
    sync::Arc,
    time::{Duration, Instant},
};

use crate::{ControlServerPlugin, RouterBuilder};

#[derive(Default)]
pub(crate) struct HealthCheckRegistry {
    health_checks: Vec<Arc<dyn DynHealthCheck>>,
    types: HashSet<TypeId>,
}

impl HealthCheckRegistry {
    pub fn add_health_check<T>(&mut self, health_check: Arc<T>)
    where
        T: HealthCheck + 'static,
    {
        if !self.types.insert(TypeId::of::<T>()) {
            panic!("Health check {} already added", type_name::<T>());
        }
        self.health_checks.push(health_check);
    }

    pub fn has_health_check<T>(&self) -> bool
    where
        T: HealthCheck + 'static,
    {
        self.types.contains(&TypeId::of::<T>())
    }

    pub fn build_health_checks(&self) -> Arc<[Arc<dyn DynHealthCheck>]> {
        self.health_checks.clone().into()
    }
}

/// A named check reporting whether a part of the application is healthy.
///
/// Implementors are aggregated by the control server's [`HealthRouter`]: a
/// `GET /health` runs every registered check in registration order and fails on
/// the first error, reporting that check's [`name`](HealthCheck::name).
///
/// Register a check with [`AddHealthCheckExt::add_health_check`] (a concrete
/// instance) or [`AddHealthCheckServiceExt::add_health_check_service`] (resolved
/// from a [`Service`]).
pub trait HealthCheck: Send + Sync {
    /// Name reported when this check fails.
    fn name(&self) -> &str;

    /// Performs the check, returning `Err` if the dependency is unhealthy.
    fn health_check(&self) -> impl Future<Output = Result<(), StdError>> + Send;
}

#[async_trait]
pub(crate) trait DynHealthCheck: Send + Sync {
    fn name(&self) -> &str;

    async fn health_check(&self) -> Result<(), StdError>;
}

#[async_trait]
impl<T> DynHealthCheck for T
where
    T: HealthCheck + 'static,
{
    fn name(&self) -> &str {
        self.name()
    }

    async fn health_check(&self) -> Result<(), StdError> {
        self.health_check().await
    }
}

struct HealthCheckServiceProvider<T>(PhantomData<T>);

impl<T> Plugin for HealthCheckServiceProvider<T>
where
    T: Service<Handle = Arc<T>> + HealthCheck + 'static,
{
    async fn build(&self, ctx: &AppContext) -> Result<(), StdError> {
        let component = ctx.get_component::<T::Handle>().unwrap();
        ctx.get_component_mut::<HealthCheckRegistry>()
            .unwrap()
            .add_health_check(component);
        Ok(())
    }

    fn dependencies(&self) -> Dependencies {
        T::dependencies()
            .service::<T>()
            .plugin::<ControlServerPlugin>()
    }
}

/// Registers concrete [`HealthCheck`] instances on the control server.
///
/// Lives on [`AppContext`], so checks can be registered while configuring the
/// [`AppBuilder`] or from within a plugin's `build`. A check is identified by its
/// type and is only ever run if [`ControlServerPlugin`] is added (it provides the
/// registry and the [`HealthRouter`] that drives the checks).
pub trait AddHealthCheckExt {
    /// Registers `health_check` on the control server.
    ///
    /// # Panics
    ///
    /// Panics if a health check of type `T` is already registered. Guard with
    /// [`has_health_check`](AddHealthCheckExt::has_health_check) when the same
    /// type may be registered more than once; to run several checks of the same
    /// kind, make the type itself aggregate them.
    fn add_health_check<T>(&self, health_check: impl Into<Arc<T>>)
    where
        T: HealthCheck + 'static;

    /// Returns whether a health check of type `T` is registered.
    fn has_health_check<T>(&self) -> bool
    where
        T: HealthCheck + 'static;
}

impl AddHealthCheckExt for AppContext {
    fn add_health_check<T>(&self, health_check: impl Into<Arc<T>>)
    where
        T: HealthCheck + 'static,
    {
        if !self.has_component::<HealthCheckRegistry>() {
            self.add_component(HealthCheckRegistry::default());
        }
        self.get_component_mut::<HealthCheckRegistry>()
            .unwrap()
            .add_health_check(health_check.into());
    }

    fn has_health_check<T>(&self) -> bool
    where
        T: HealthCheck + 'static,
    {
        self.get_component_ref::<HealthCheckRegistry>()
            .is_some_and(|registry| registry.has_health_check::<T>())
    }
}

/// Registers health checks resolved from the dependency-injection container.
///
/// The check type `T` is a [`Service`], built by the container and registered as
/// a [`HealthCheck`]. The service is added automatically if not already present.
pub trait AddHealthCheckServiceExt {
    /// Registers the [`Service`] `T` as a health check on the control server.
    ///
    /// # Panics
    ///
    /// Panics if `T` is already registered as a health-check service. Building
    /// the [`App`] additionally panics if `T` is registered both as a service
    /// and as an instance via [`AddHealthCheckExt::add_health_check`].
    fn add_health_check_service<T>(&mut self) -> &mut Self
    where
        T: Service<Handle = Arc<T>> + HealthCheck + 'static;

    /// Returns whether `T` is registered as a health-check service.
    fn has_health_check_service<T>(&self) -> bool
    where
        T: Service<Handle = Arc<T>> + HealthCheck + 'static;
}

impl AddHealthCheckServiceExt for AppBuilder {
    fn add_health_check_service<T>(&mut self) -> &mut Self
    where
        T: Service<Handle = Arc<T>> + HealthCheck + 'static,
    {
        if !self.has_service::<T>() {
            self.add_service::<T>();
        }
        self.add_plugin(HealthCheckServiceProvider::<T>(PhantomData));
        self
    }

    fn has_health_check_service<T>(&self) -> bool
    where
        T: Service<Handle = Arc<T>> + HealthCheck + 'static,
    {
        self.has_plugin::<HealthCheckServiceProvider<T>>()
    }
}

/// Client for probing a service's `/health` endpoint over HTTP.
///
/// The [`ControlServerPlugin`] registers a `HealthClient` component pointed at
/// its own health endpoint; it can also be built directly with
/// [`new`](HealthClient::new) to probe a remote service (for example to wait for
/// a dependency to become ready).
#[derive(Clone)]
pub struct HealthClient {
    client: reqwest::Client,
    endpoint: String,
}

impl HealthClient {
    /// Creates a client that probes `endpoint`.
    ///
    /// `endpoint` must be the full health URL, for example
    /// `http://127.0.0.1:8080/health`.
    pub fn new(endpoint: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            endpoint,
        }
    }

    /// Performs a single health probe.
    ///
    /// # Errors
    ///
    /// Returns a [`HealthCheckError`] if the request cannot be sent or the
    /// endpoint responds with a non-success status.
    pub async fn health_check(&self) -> Result<(), HealthCheckError> {
        let response = self
            .client
            .get(&self.endpoint)
            .send()
            .await
            .map_err(|err| HealthCheckError {
                name: "health_client".into(),
                message: format!("Health check failed: {err}"),
            })?;
        let status = response.status();
        if status.is_success() {
            Ok(())
        } else {
            Err(response.json().await.map_err(|_| HealthCheckError {
                name: "health_client".into(),
                message: format!("Health check failed with status: {status}"),
            })?)
        }
    }

    /// Polls the endpoint until it reports healthy or `timeout` elapses,
    /// retrying every 100 ms.
    ///
    /// # Errors
    ///
    /// Returns the last [`HealthCheckError`] if the endpoint is still not healthy
    /// when `timeout` elapses.
    pub async fn wait_for_ready(&self, timeout: Duration) -> Result<(), HealthCheckError> {
        let start = Instant::now();
        loop {
            match self.health_check().await {
                Ok(_) => return Ok(()),
                Err(err) => {
                    if start.elapsed() >= timeout {
                        return Err(err);
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        }
    }
}

/// Router exposing `GET /health` on the control server.
///
/// Aggregates every registered [`HealthCheck`]: the endpoint returns `200` with
/// body `healthy` when all checks pass, or `500` with a JSON [`HealthCheckError`]
/// naming the first check that failed. Register it with
/// [`add_control_router_service`](crate::AddControlRouterServiceExt::add_control_router_service);
/// it relies on [`ControlServerPlugin`] for the health-check registry.
#[derive(Service)]
pub struct HealthRouter;

impl RouterBuilder for HealthRouter {
    fn build_router(self: Arc<Self>, app: &App) -> Router {
        let health_checks = app
            .get_component_ref::<HealthCheckRegistry>()
            .unwrap()
            .build_health_checks();
        Router::new().route(
            "/health",
            routing::get(|| async move { self.health(health_checks.as_ref()).await }),
        )
    }
}

const HEALTHY: &str = "healthy";

impl HealthRouter {
    async fn health(
        &self,
        health_checks: &[Arc<dyn DynHealthCheck>],
    ) -> Result<&'static str, HealthCheckError> {
        for health_check in health_checks {
            let name = health_check.name().to_string();
            if let Err(err) = health_check.health_check().await {
                return Err(HealthCheckError {
                    name,
                    message: err.to_string(),
                });
            }
        }
        Ok(HEALTHY)
    }
}

/// Error reported by a failed health check: the failing check's name and a
/// message.
///
/// Serializes to JSON and renders as an HTTP `500` response.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HealthCheckError {
    name: String,
    message: String,
}

impl axum::response::IntoResponse for HealthCheckError {
    fn into_response(self) -> axum::response::Response {
        (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            axum::response::Json(self),
        )
            .into_response()
    }
}

/// Router exposing `GET /ping`, which always returns `pong`.
///
/// A trivial liveness endpoint; register it as a router on either server.
#[derive(Service)]
pub struct PingHandler;

impl PingHandler {
    async fn ping() -> &'static str {
        "pong"
    }
}

impl RouterBuilder for PingHandler {
    fn build_router(self: Arc<Self>, _app: &App) -> Router {
        Router::new().route("/ping", routing::get(|| async move { Self::ping().await }))
    }
}
