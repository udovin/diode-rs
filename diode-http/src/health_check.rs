use async_trait::async_trait;
use axum::{Router, routing};
use diode::{
    AddServiceExt as _, App, AppBuilder, Dependencies, Plugin, Service, ServiceDependencyExt as _,
    StdError,
};
use serde::{Deserialize, Serialize};
use std::{
    marker::PhantomData,
    sync::Arc,
    time::{Duration, Instant},
};

use crate::{RouterBuilder, ServiceServerPlugin};

#[derive(Default)]
pub(crate) struct HealthCheckRegistry {
    health_checks: Vec<Arc<dyn DynHealthCheck>>,
}

#[allow(unused)]
impl HealthCheckRegistry {
    pub fn add_health_check<T>(&mut self, health_check: Arc<T>)
    where
        T: HealthCheck + 'static,
    {
        self.health_checks.push(health_check);
    }

    pub fn build_health_checks(&self) -> Arc<[Arc<dyn DynHealthCheck>]> {
        self.health_checks.clone().into()
    }
}

pub trait HealthCheck: Send + Sync {
    fn name(&self) -> &str;

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

struct HealthCheckProvider<T>(PhantomData<T>);

impl<T> Plugin for HealthCheckProvider<T>
where
    T: Service<Handle = Arc<T>> + HealthCheck + 'static,
{
    async fn build(&self, app: &mut AppBuilder) -> Result<(), StdError> {
        let component = app.get_component::<T::Handle>().unwrap();
        app.get_component_mut::<HealthCheckRegistry>()
            .unwrap()
            .add_health_check(component);
        Ok(())
    }

    fn dependencies(&self) -> Dependencies {
        T::dependencies()
            .service::<T>()
            .plugin::<ServiceServerPlugin>()
    }
}

pub trait AddHealthCheckExt {
    fn add_health_check<T>(&mut self) -> &mut Self
    where
        T: Service<Handle = Arc<T>> + HealthCheck + 'static;

    fn has_health_check<T>(&self) -> bool
    where
        T: Service<Handle = Arc<T>> + HealthCheck + 'static;
}

impl AddHealthCheckExt for AppBuilder {
    fn add_health_check<T>(&mut self) -> &mut Self
    where
        T: Service<Handle = Arc<T>> + HealthCheck + 'static,
    {
        if !self.has_service::<T>() {
            self.add_service::<T>();
        }
        self.add_plugin(HealthCheckProvider::<T>(PhantomData));
        self
    }

    fn has_health_check<T>(&self) -> bool
    where
        T: Service<Handle = Arc<T>> + HealthCheck + 'static,
    {
        self.has_plugin::<HealthCheckProvider<T>>()
    }
}

#[derive(Clone)]
pub struct HealthClient {
    client: reqwest::Client,
    endpoint: String,
}

impl HealthClient {
    pub fn new(endpoint: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            endpoint,
        }
    }

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
