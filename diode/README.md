# diode

A small dependency-injection framework for async Rust. `diode` is the core
crate: an application container with type-keyed components and a plugin/service
model that initializes everything in dependency order.

It is the foundation of the `diode-rs` workspace, with `diode-base` (config,
daemons, CLI, tracing) and `diode-http` (HTTP servers) built on top.

```toml
[dependencies]
diode = "0.2"
```

## Concepts

- **Component** - any `Send + Sync + 'static` value, stored and looked up by type.
- **Service** - a component with async initialization and declared dependencies;
  derive it with `#[derive(Service)]`.
- **Plugin** - build-time logic that registers components, services, or daemons.
- **App / AppBuilder** - configure a builder, `build().await`, get an `App`.

## Example

`#[derive(Service)]` builds the service and injects its fields from the
container: `#[inject(Component)]` pulls a plain component (here `Config`), while
`Arc<OtherService>` fields are resolved as service handles.

```rust
use diode::{AddServiceExt, App, Component, Service};
use std::sync::Arc;

#[derive(Clone)]
struct Config {
    database_url: String,
}

#[derive(Service)]
struct Database {
    #[inject(Component)]
    config: Config,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let app = App::builder()
        .add_component(Config {
            database_url: "postgres://localhost/db".to_string(),
        })
        .add_service::<Database>()
        .build()
        .await?;

    let db = app.get_component::<Arc<Database>>().unwrap();
    println!("{}", db.config.database_url);
    Ok(())
}
```

The builder topologically sorts everything by its declared dependencies and runs
each `build` once, reporting cycles and missing dependencies.

## Features

- `macros` (default) - `#[derive(Service)]` and field injection.

## License

Licensed under either of MIT or Apache-2.0 at your option.
