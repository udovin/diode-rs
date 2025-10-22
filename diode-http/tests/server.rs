use std::convert::Infallible;

use axum::http::status::StatusCode;
use axum::response::IntoResponse;
use diode_base::test::FreePort;
use reqwest_middleware::ClientBuilder;
use reqwest_retry::{RetryTransientMiddleware, policies::ExponentialBackoff};

use diode::{App, Service};
use diode_base::{CancellationToken, Config, RunDaemonsExt as _};
use diode_http::{
    AddHealthCheckExt, AddMiddlewareExt, AddRouterExt, AddServiceRouterExt as _, HealthCheck,
    HealthRouter, HttpServerConfig, HttpServerPlugin, MiddlewareService, Next, Request, Response,
    ServiceServerConfig, ServiceServerPlugin, router,
};

#[derive(Service)]
pub struct ExampleRouter;

#[router(middleware = [ReqIdMiddleware])]
impl ExampleRouter {
    #[route(get, path = "/public")]
    async fn public(&self) -> String {
        "public value".to_string()
    }

    #[route(get, path = "/private", middleware = [AuthMiddleware])]
    async fn private(&self) -> String {
        "private value".to_string()
    }
}

#[derive(Service)]
pub struct AuthMiddleware;

impl MiddlewareService for AuthMiddleware {
    type Error = Infallible;

    async fn call(&self, request: Request, next: impl Next) -> Result<Response, Infallible> {
        let auth = str::from_utf8(
            request
                .headers()
                .get("Authorization")
                .map(|v| v.as_bytes())
                .unwrap_or("".as_bytes()),
        )
        .unwrap();
        if auth != "password" {
            return Ok(StatusCode::UNAUTHORIZED.into_response());
        }
        return Ok(next.call(request).await);
    }
}

#[derive(Service)]
pub struct ReqIdMiddleware;

impl MiddlewareService for ReqIdMiddleware {
    type Error = Infallible;

    async fn call(&self, request: Request, next: impl Next) -> Result<Response, Infallible> {
        let mut response = next.call(request).await;
        response
            .headers_mut()
            .append("X-Req-Id", "abacaba".parse().unwrap());
        Ok(response)
    }
}

#[tokio::test]
async fn test_example_router_and_middleware() {
    let server_port = FreePort::new();

    let app = App::builder()
        .add_plugin(HttpServerPlugin)
        .add_router::<ExampleRouter>()
        .add_middleware::<AuthMiddleware>()
        .add_middleware::<ReqIdMiddleware>()
        .add_component(Config::new().with(
            "http_server",
            HttpServerConfig {
                addr: server_port.as_addr(),
            },
        ))
        .build()
        .await
        .unwrap();

    let shutdown = CancellationToken::new();

    let shutdown_clone = shutdown.clone();
    let server_task = tokio::spawn(async move { app.run_daemons(shutdown_clone).await });

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let retry_policy = ExponentialBackoff::builder().build_with_max_retries(3);
    let client = ClientBuilder::new(reqwest::Client::new())
        .with(RetryTransientMiddleware::new_with_policy(retry_policy))
        .build();

    let base_url = format!("http://{}", server_port.as_addr());

    let response = client
        .get(&format!("{}/public", base_url))
        .send()
        .await
        .expect("Failed to send request");

    assert_eq!(response.status(), 200);
    assert!(response.headers().contains_key("X-Req-Id"));
    let body = response.text().await.expect("Failed to read response body");
    assert_eq!(body, "public value");

    let response = client
        .get(&format!("{}/private", base_url))
        .header("Authorization", "password")
        .send()
        .await
        .expect("Failed to send request with header");

    assert_eq!(response.status(), 200);
    assert!(response.headers().contains_key("X-Req-Id"));
    let body = response.text().await.expect("Failed to read response body");
    assert_eq!(body, "private value");

    let response = client
        .get(&format!("{}/private", base_url))
        .send()
        .await
        .expect("Failed to send request with different header");

    assert_eq!(response.status(), 401);
    assert!(response.headers().contains_key("X-Req-Id"));

    shutdown.cancel();
    let _ = tokio::time::timeout(tokio::time::Duration::from_secs(5), server_task).await;
}

#[tokio::test]
async fn test_service_server() {
    let server_port = FreePort::new();

    let app = App::builder()
        .add_plugin(ServiceServerPlugin)
        .add_service_router::<HealthRouter>()
        .add_component(Config::new().with(
            "service_http_server",
            ServiceServerConfig {
                addr: server_port.as_addr(),
            },
        ))
        .build()
        .await
        .unwrap();

    let shutdown = CancellationToken::new();

    let shutdown_clone = shutdown.clone();
    let server_task = tokio::spawn(async move { app.run_daemons(shutdown_clone).await });

    let retry_policy = ExponentialBackoff::builder().build_with_max_retries(3);
    let client = ClientBuilder::new(reqwest::Client::new())
        .with(RetryTransientMiddleware::new_with_policy(retry_policy))
        .build();

    let base_url = format!("http://{}", server_port.as_addr());

    let response = client
        .get(&format!("{}/health", base_url))
        .send()
        .await
        .expect("Failed to send request");

    assert_eq!(response.status(), 200);
    let body = response.text().await.expect("Failed to read response body");
    assert_eq!(body, "healthy");

    shutdown.cancel();
    let _ = tokio::time::timeout(tokio::time::Duration::from_secs(5), server_task).await;
}

#[derive(Service)]
struct BadHealthCheckService;

impl HealthCheck for BadHealthCheckService {
    fn name(&self) -> &str {
        "bad_health_check"
    }

    async fn health_check(&self) -> Result<(), diode::StdError> {
        Err("Bad health check".into())
    }
}

#[tokio::test]
async fn test_unhealthy_service() {
    let server_port = FreePort::new();

    let app = App::builder()
        .add_plugin(ServiceServerPlugin)
        .add_service_router::<HealthRouter>()
        .add_health_check::<BadHealthCheckService>()
        .add_component(Config::new().with(
            "service_http_server",
            ServiceServerConfig {
                addr: server_port.as_addr(),
            },
        ))
        .build()
        .await
        .unwrap();

    let shutdown = CancellationToken::new();

    let shutdown_clone = shutdown.clone();
    let server_task = tokio::spawn(async move { app.run_daemons(shutdown_clone).await });

    let retry_policy = ExponentialBackoff::builder().build_with_max_retries(3);
    let client = ClientBuilder::new(reqwest::Client::new())
        .with(RetryTransientMiddleware::new_with_policy(retry_policy))
        .build();

    let base_url = format!("http://{}", server_port.as_addr());

    let response = client
        .get(&format!("{}/health", base_url))
        .send()
        .await
        .expect("Failed to send request");

    assert_eq!(response.status(), 500);
    let body = response.text().await.expect("Failed to read response body");
    assert_eq!(
        body,
        "{\"name\":\"bad_health_check\",\"message\":\"Bad health check\"}"
    );

    shutdown.cancel();
    let _ = tokio::time::timeout(tokio::time::Duration::from_secs(5), server_task).await;
}
