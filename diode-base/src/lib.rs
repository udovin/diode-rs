//! # diode-base
//!
//! Base functionality and utilities for diode applications, providing essential
//! building blocks for creating robust, configurable, and maintainable services.
//!
//! This crate extends the core diode dependency injection framework with
//! practical utilities for real-world applications including configuration management,
//! daemon services, command-line interfaces, and application lifecycle management.
//!
//! ## Core Components
//!
//! - **Configuration System**: Type-safe configuration loading and merging from multiple sources
//! - **Daemon Framework**: Long-running background services with graceful shutdown
//! - **Command System**: CLI framework with subcommands and dependency injection
//! - **Bundle Management**: Modular application component grouping
//! - **Tracing Integration**: Structured logging and observability
//! - **Dynamic Configuration**: Runtime configuration updates and hot-reloading
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use diode::App;
//! use diode_base::{RunMainExt, AddCommandExt, Command};
//! use std::process::ExitCode;
//! use std::sync::Arc;
//! use clap::Command as ClapCommand;
//!
//! struct HelloCommand;
//!
//! impl Command for HelloCommand {
//!     fn command() -> ClapCommand {
//!         ClapCommand::new("hello").about("Says hello")
//!     }
//!
//!     async fn main(_app: Arc<App>, _matches: clap::ArgMatches) -> ExitCode {
//!         println!("Hello from diode!");
//!         ExitCode::SUCCESS
//!     }
//! }
//!
//! #[tokio::main]
//! async fn main() -> ExitCode {
//!     App::builder()
//!         .add_command::<HelloCommand>()
//!         .run_main()
//!         .await
//! }
//! ```
//!
//! ## Configuration Example
//!
//! ```rust
//! use diode::App;
//! use diode_base::Config;
//! use serde::{Deserialize, Serialize};
//!
//! #[derive(Debug, Serialize, Deserialize)]
//! struct DatabaseConfig {
//!     host: String,
//!     port: u16,
//! }
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
//! let app = App::builder()
//!     .add_component(Config::new().with("database", DatabaseConfig {
//!         host: "localhost".to_string(),
//!         port: 5432,
//!     }))
//!     .build()
//!     .await?;
//!
//! let config = app.get_component_ref::<Config>().unwrap();
//! let db_config = config.get::<DatabaseConfig>("database")?;
//! println!("Database: {}:{}", db_config.host, db_config.port);
//! # Ok(())
//! # }
//! ```
//!
//! ## Features
//!
//! - `macros` (default): Enables procedural macros for simplified configuration and service definitions

mod bundle;
mod command;
mod config;
mod daemon;
mod defer;
mod dynamic_config;
mod dynamic_config_file;
mod metrics;
pub mod test;
mod tracing;

pub use bundle::*;
pub use command::*;
pub use config::*;
pub use daemon::*;
pub use defer::*;
pub use dynamic_config::*;
pub use dynamic_config_file::*;
pub use metrics::*;
pub use tracing::*;

#[cfg(feature = "macros")]
pub use diode_base_macros::*;

pub use async_trait::async_trait;
