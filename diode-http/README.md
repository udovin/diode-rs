# diode-http

HTTP services for the [`diode`](https://crates.io/crates/diode)
dependency-injection framework, built on [`axum`](https://crates.io/crates/axum).
It provides HTTP servers, attribute macros for declaring routers, dependency-
injected middleware, and ready-made health endpoints.

```toml
[dependencies]
diode = "0.2"
diode-base = "0.3"
diode-http = "0.3"
```

## Example

Declare a router with the `#[router]` / `#[route]` macros, register it, and run
the server:

```rust
use diode::{App, Service};
use diode_base::{CancellationToken, Config, RunDaemonsExt};
use diode_http::{router, AddRouterServiceExt, HttpServerConfig, HttpServerPlugin};

#[derive(Service)]
struct Api;

#[router]
impl Api {
    #[route(get, path = "/hello")]
    async fn hello(&self) -> String {
        "hello, world".to_string()
    }
}

#[tokio::main]
async fn main() {
    let app = App::builder()
        .add_plugin(HttpServerPlugin)
        .add_router_service::<Api>()
        .add_component(Config::new().with(
            "http_server",
            HttpServerConfig { addr: "127.0.0.1:8080".parse().unwrap() },
        ))
        .build()
        .await
        .unwrap();

    app.run_daemons(CancellationToken::new()).await.unwrap();
}
```

## Servers

Two independent servers are available, each running as a daemon and shutting
down gracefully:

- **Public** - `HttpServerPlugin`, configured from the `http_server` section.
- **Control** - `ControlServerPlugin`, configured from the `control_server`
  section. A separate, typically internal, plane for operational endpoints; it
  hosts the health-check registry and a `HealthClient` pointed at its own
  `/health`.

## Routers

A router is any type implementing `RouterBuilder`. The easiest way is the
`#[router]` attribute on an `impl` block, with `#[route(method, path = "..")]`
on each handler. Register routers either as a concrete instance or resolved from
the DI container:

| | public server | control server |
| --- | --- | --- |
| instance | `add_router(value)` | `add_control_router(value)` |
| DI service | `add_router_service::<T>()` | `add_control_router_service::<T>()` |

Each type may back at most one router; registering the same type twice panics.
`has_router` / `has_router_service` (and the control-server equivalents) let you
check first.

## Middleware

Middleware implements the `Middleware` trait. Register a concrete instance with
`add_middleware(value)`, or a DI-built middleware (one that holds injected
dependencies) with `add_middleware_service::<T>()`. Attach it with the macro
attributes:

```rust,ignore
#[router(middleware = [RequestId])]
impl Api {
    #[route(get, path = "/private", middleware = [Auth])]
    async fn private(&self) -> String { /* ... */ }
}
```

Ordering: within a `middleware = [A, B]` list the first entry is outermost (runs
first on the request, last on the response), and router-level middleware wraps
route-level middleware.

## Health checks

Implement `HealthCheck` and register it with `add_health_check(..)` /
`add_health_check_service::<T>()`. Add `HealthRouter` to the control server to
expose `GET /health`, which runs every registered check and returns `200`
`healthy` or `500` with a JSON error naming the first failing check. `PingHandler`
exposes a trivial `GET /ping`, and `HealthClient` probes a `/health` endpoint
(useful for readiness waits).

## Features

- `macros` (default) - the `#[router]` / `#[route]` attribute macros.

## License

Licensed under either of MIT or Apache-2.0 at your option.
