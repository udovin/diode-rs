//! # diode
//!
//! A dependency injection framework for Rust applications that provides type-safe,
//! async-compatible dependency management with plugin-based architecture.
//!
//! ## Core Concepts
//!
//! - **App**: The main container that holds all registered components and services
//! - **Service**: A trait for defining injectable services with async initialization
//! - **Plugin**: A trait for modular components that can register services and dependencies
//! - **Components**: Raw objects stored in the app container
//! - **Dependencies**: Type-safe dependency declarations between services and plugins
//!
//! ## Basic Usage
//!
//! Simple service registration and retrieval:
//!
//! ```rust
//! use diode::{App, Service, StdError, AddServiceExt};
//! use std::sync::Arc;
//!
//! struct DatabaseService {
//!     connection_string: String,
//! }
//!
//! impl Service for DatabaseService {
//!     type Handle = Arc<Self>;
//!
//!     async fn build(_app: &diode::AppBuilder) -> Result<Self::Handle, StdError> {
//!         Ok(Arc::new(Self {
//!             connection_string: "sqlite::memory:".to_string(),
//!         }))
//!     }
//! }
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let app = App::builder()
//!         .add_service::<DatabaseService>()
//!         .build()
//!         .await?;
//!
//!     let db = app.get_component::<Arc<DatabaseService>>().unwrap();
//!     println!("Database connected: {}", db.connection_string);
//!     Ok(())
//! }
//! ```
//!
//! ## Service Dependencies
//!
//! Services can depend on other services, with automatic dependency resolution:
//!
//! ```rust
//! use diode::{App, Service, StdError, Dependencies, ServiceDependencyExt, AddServiceExt};
//! use std::sync::Arc;
//!
//! struct ConfigService {
//!     database_url: String,
//! }
//!
//! struct DatabaseService {
//!     config: Arc<ConfigService>,
//! }
//!
//! struct ApiService {
//!     database: Arc<DatabaseService>,
//! }
//!
//! impl Service for ConfigService {
//!     type Handle = Arc<Self>;
//!
//!     async fn build(_app: &diode::AppBuilder) -> Result<Self::Handle, StdError> {
//!         Ok(Arc::new(Self {
//!             database_url: "postgresql://localhost:5432/mydb".to_string(),
//!         }))
//!     }
//! }
//!
//! impl Service for DatabaseService {
//!     type Handle = Arc<Self>;
//!
//!     async fn build(app: &diode::AppBuilder) -> Result<Self::Handle, StdError> {
//!         let config = app.get_component::<Arc<ConfigService>>()
//!             .ok_or("ConfigService not found")?;
//!
//!         Ok(Arc::new(Self { config }))
//!     }
//!
//!     fn dependencies() -> Dependencies {
//!         Dependencies::new().service::<ConfigService>()
//!     }
//! }
//!
//! impl Service for ApiService {
//!     type Handle = Arc<Self>;
//!
//!     async fn build(app: &diode::AppBuilder) -> Result<Self::Handle, StdError> {
//!         let database = app.get_component::<Arc<DatabaseService>>()
//!             .ok_or("DatabaseService not found")?;
//!
//!         Ok(Arc::new(Self { database }))
//!     }
//!
//!     fn dependencies() -> Dependencies {
//!         Dependencies::new().service::<DatabaseService>()
//!     }
//! }
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let app = App::builder()
//!         .add_service::<ConfigService>()
//!         .add_service::<DatabaseService>()
//!         .add_service::<ApiService>()
//!         .build()
//!         .await?;
//!
//!     let api = app.get_component::<Arc<ApiService>>().unwrap();
//!     println!("API service initialized with database URL: {}",
//!              api.database.config.database_url);
//!     Ok(())
//! }
//! ```
//!
//! ## Plugin System
//!
//! For more complex initialization logic, use plugins:
//!
//! ```rust
//! use diode::{App, Plugin, Dependencies, StdError, AppBuilder};
//!
//! struct DatabasePlugin {
//!     connection_string: String,
//! }
//!
//! impl Plugin for DatabasePlugin {
//!     async fn build(&self, app: &mut AppBuilder) -> Result<(), StdError> {
//!         // Register components or perform complex initialization
//!         app.add_component(self.connection_string.clone());
//!         Ok(())
//!     }
//! }
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let app = App::builder()
//!         .add_plugin(DatabasePlugin {
//!             connection_string: "postgresql://localhost:5432/mydb".to_string(),
//!         })
//!         .build()
//!         .await?;
//!
//!     let connection_string = app.get_component::<String>().unwrap();
//!     println!("Database connection: {}", connection_string);
//!     Ok(())
//! }
//! ```
//!
//! ## Using Macros
//!
//! With the `macros` feature enabled, service definition becomes much simpler:
//!
//! ```rust
//! use diode::{App, AddServiceExt, Component, Service};
//! use std::sync::Arc;
//!
//! #[derive(Service)]
//! struct DatabaseService {
//!     #[inject(Component)]
//!     connection_string: String,
//! }
//!
//! #[derive(Service)]
//! struct ApiService {
//!     database: Arc<DatabaseService>,
//! }
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let app = App::builder()
//!         .add_component("postgresql://localhost:5432/mydb".to_string())
//!         .add_service::<DatabaseService>()
//!         .add_service::<ApiService>()
//!         .build()
//!         .await?;
//!
//!     let api = app.get_component::<Arc<ApiService>>().unwrap();
//!     println!("Services initialized successfully");
//!     Ok(())
//! }
//! ```
//!
//! ## Features
//!
//! - `macros` (default): Enables procedural macros for simplified service definitions

mod app;
mod inject;
mod service;

pub use app::*;
pub use inject::*;
pub use service::*;

#[cfg(feature = "macros")]
pub use diode_macros::*;
