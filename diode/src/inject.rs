//! Dependency injection utilities for extracting components and services from the application.
//!
//! This module provides traits and types for implementing dependency injection patterns,
//! allowing services and plugins to declare their dependencies and extract them from
//! the application builder in a type-safe manner.
//!
//! # Core Traits
//!
//! - [`Dependency`] - Declares dependencies for extraction
//! - [`Extract`] - Extracts owned values from the application
//! - [`ExtractRef`] - Extracts borrowed references from the application
//!
//! # Examples
//!
//! Extracting a service dependency:
//!
//! ```rust
//! use diode::{Service, AppBuilder, StdError, Extract};
//! use std::sync::Arc;
//!
//! struct DatabaseService;
//! struct ApiService;
//!
//! impl Service for DatabaseService {
//!     type Handle = Arc<Self>;
//!     async fn build(_app: &AppBuilder) -> Result<Self::Handle, StdError> {
//!         Ok(Arc::new(Self))
//!     }
//! }
//!
//! impl Service for ApiService {
//!     type Handle = Arc<Self>;
//!     async fn build(app: &AppBuilder) -> Result<Self::Handle, StdError> {
//!         let db = DatabaseService::extract(app)?;
//!         Ok(Arc::new(Self))
//!     }
//! }
//! ```

use crate::{AppBuilder, AppError, Dependencies, Service, ServiceDependencyExt};

/// Trait for extracting owned values from the application builder.
///
/// This trait allows extracting components or service handles from the application
/// by value. The extracted value must implement `Clone` to be extracted this way.
///
/// # Type Parameters
///
/// * `T` - The type to extract from the application.
///
/// # Examples
///
/// ```rust
/// use diode::{AppBuilder, AppError, Extract};
///
/// struct ConfigExtractor;
///
/// impl Extract<String> for ConfigExtractor {
///     fn extract(app: &AppBuilder) -> Result<String, AppError> {
///         app.get_component::<String>()
///             .ok_or(AppError::MissingDependency)
///     }
/// }
/// ```
pub trait Extract<T> {
    /// Extracts a value of type `T` from the application builder.
    ///
    /// # Arguments
    ///
    /// * `app` - Reference to the application builder containing components.
    ///
    /// # Returns
    ///
    /// Returns `Ok(T)` if the component exists and can be extracted,
    /// or `Err(AppError)` if extraction fails.
    fn extract(app: &AppBuilder) -> Result<T, AppError>;

    /// Returns the dependencies required by this type.
    ///
    /// The default implementation returns an empty dependencies set,
    /// indicating no dependencies are required.
    ///
    /// # Returns
    ///
    /// A `Dependencies` object describing required dependencies.
    fn dependencies() -> Dependencies {
        Dependencies::new()
    }
}

/// Trait for extracting borrowed references from the application builder.
///
/// This trait allows extracting components or service handles from the application
/// by reference, avoiding the need for cloning. This is more efficient when you
/// only need to read from the component.
///
/// # Type Parameters
///
/// * `T` - The type to extract a reference to from the application.
///
/// # Examples
///
/// ```rust
/// use diode::{AppBuilder, AppError, ExtractRef};
///
/// struct ConfigExtractor;
///
/// impl ExtractRef<String> for ConfigExtractor {
///     fn extract_ref<'a>(app: &'a AppBuilder) -> Result<&'a String, AppError> {
///         app.get_component_ref::<String>()
///             .ok_or(AppError::MissingDependency)
///     }
/// }
/// ```
pub trait ExtractRef<T> {
    /// Extracts a reference to a value of type `T` from the application builder.
    ///
    /// # Arguments
    ///
    /// * `app` - Reference to the application builder containing components.
    ///
    /// # Returns
    ///
    /// Returns `Ok(&T)` if the component exists and can be referenced,
    /// or `Err(AppError)` if extraction fails.
    fn extract_ref(app: &AppBuilder) -> Result<&T, AppError>;

    /// Returns the dependencies required by this type.
    ///
    /// The default implementation returns an empty dependencies set,
    /// indicating no dependencies are required.
    ///
    /// # Returns
    ///
    /// A `Dependencies` object describing required dependencies.
    fn dependencies() -> Dependencies {
        Dependencies::new()
    }
}

impl<T, S> Extract<T> for S
where
    T: Clone + Send + Sync + 'static,
    S: Service<Handle = T> + 'static,
{
    fn extract(app: &AppBuilder) -> Result<T, AppError> {
        Ok(app.get_component::<T>().unwrap())
    }

    fn dependencies() -> Dependencies {
        Dependencies::new().service::<S>()
    }
}

impl<T, S> ExtractRef<T> for S
where
    T: Send + Sync + 'static,
    S: Service<Handle = T> + 'static,
{
    fn extract_ref(app: &AppBuilder) -> Result<&T, AppError> {
        Ok(app.get_component_ref::<T>().unwrap())
    }

    fn dependencies() -> Dependencies {
        Dependencies::new().service::<S>()
    }
}

impl ExtractRef<AppBuilder> for AppBuilder {
    fn extract_ref(app: &AppBuilder) -> Result<&AppBuilder, AppError> {
        Ok(app)
    }
}

/// Generic extractor for any component stored in the application.
///
/// This zero-sized type can be used to extract any component that has been
/// registered with the application builder using `add_component()`.
///
/// # Examples
///
/// ```rust
/// use diode::{App, Component, Extract};
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let app = App::builder()
///     .add_component("configuration".to_string())
///     .build()
///     .await?;
///
/// // During service building:
/// // let config = Component::extract::<String>(app_builder)?;
/// # Ok(())
/// # }
/// ```
pub struct Component;

impl<T> Extract<T> for Component
where
    T: Clone + Send + Sync + 'static,
{
    fn extract(app: &AppBuilder) -> Result<T, AppError> {
        app.get_component::<T>().ok_or(AppError::MissingDependency)
    }
}

impl<T> ExtractRef<T> for Component
where
    T: Send + Sync + 'static,
{
    fn extract_ref(app: &AppBuilder) -> Result<&T, AppError> {
        app.get_component_ref::<T>()
            .ok_or(AppError::MissingDependency)
    }
}
