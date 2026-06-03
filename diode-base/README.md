# diode-base

Application building blocks for the [`diode`](https://crates.io/crates/diode)
dependency-injection framework: layered configuration, long-running daemons, a
CLI entry point, structured tracing and metrics, and dynamic configuration.

```toml
[dependencies]
diode = "0.2"
diode-base = "0.3"
```

## What it provides

- **Configuration** - `Config` loads and merges layered JSON. Declare a typed
  section with `#[config_section("name")]` and read it with `config.get`.
- **Daemons** - the `Daemon` trait, `AddDaemonExt` / `AddDaemonServiceExt` to
  register background tasks, and `RunDaemonsExt::run_daemons` to run them
  concurrently with cooperative, token-based shutdown.
- **CLI** - the `Command` trait, `AddCommandExt`, and `RunMainExt::run_main`,
  which parses arguments, loads config, sets up tracing/metrics, builds the app,
  and dispatches a subcommand. Built-in `server` runs every daemon; `config`
  prints the resolved configuration.
- **Observability** - `Tracing` and `Metrics` wire up `tracing` and OpenTelemetry
  (OTLP) exporters from the `tracing` / `metrics` config sections.
- **Dynamic configuration** - watch config sources and react to changes at
  runtime (for example to change the tracing level live).
- **Testing** - the `testing` module ships integration-test helpers such as
  `FreePort`.

## Example

A worker service driven by configuration, wired as a daemon and started through
the CLI entry point:

```rust
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use diode::{App, AppContext, Service, StdError};
use diode_base::{
    config_section, AddDaemonServiceExt, CancellationToken, Config, Daemon, RunMainExt,
};
use serde::{Deserialize, Serialize};

// A typed configuration section, read from the `worker` key of the config file.
#[derive(Serialize, Deserialize)]
#[config_section("worker")]
struct WorkerConfig {
    interval_secs: u64,
}

// A background worker: a `Service` (built from the container, reading its
// config) that is also a `Daemon` (runs until shutdown).
struct Worker {
    interval: Duration,
}

impl Service for Worker {
    type Handle = Arc<Self>;

    async fn build(ctx: &AppContext) -> Result<Self::Handle, StdError> {
        let config: WorkerConfig = ctx
            .get_component_ref::<Config>()
            .ok_or("config component missing")?
            .get("worker")?;
        Ok(Arc::new(Self {
            interval: Duration::from_secs(config.interval_secs),
        }))
    }
}

impl Daemon for Worker {
    async fn run(&self, _app: &App, shutdown: CancellationToken) -> Result<(), StdError> {
        loop {
            tokio::select! {
                _ = shutdown.cancelled() => break,
                _ = tokio::time::sleep(self.interval) => tracing::info!("tick"),
            }
        }
        Ok(())
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let mut builder = App::builder();
    builder.add_daemon_service::<Worker>();
    // `run_main` parses CLI args, loads the config file, sets up tracing and
    // metrics, then runs the selected command (the built-in `server` command
    // runs every registered daemon until Ctrl-C).
    builder.run_main().await
}
```

## Features

- `macros` (default) - the `#[config_section(..)]` attribute macro.

## License

Licensed under either of MIT or Apache-2.0 at your option.
