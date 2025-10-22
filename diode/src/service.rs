use std::marker::PhantomData;

use crate::{AppBuilder, Dependencies, Plugin};

/// Type alias for boxed errors that can be sent across threads.
///
/// This is used as the standard error type throughout the diode framework
/// for operations that can fail during service initialization or plugin building.
pub type StdError = Box<dyn std::error::Error + Send + Sync>;

/// Trait for defining injectable services with async initialization and dependency management.
///
/// Services are the primary abstraction for business logic components in diode applications.
/// They define how to build themselves asynchronously, what dependencies they require, and what
/// handle type they expose to other parts of the application.
///
/// # Type Parameters
///
/// * `Handle` - The type that represents this service when injected into other services.
///   Typically `Arc<Self>` for shared ownership, but can be any type that represents
///   the service interface.
///
/// # Examples
///
/// Basic service implementation:
///
/// ```rust
/// use diode::{Service, AppBuilder, StdError};
/// use std::sync::Arc;
///
/// struct DatabaseService {
///     connection_pool: String,
/// }
///
/// impl Service for DatabaseService {
///     type Handle = Arc<Self>;
///
///     async fn build(_app: &AppBuilder) -> Result<Self::Handle, StdError> {
///         Ok(Arc::new(Self {
///             connection_pool: "sqlite::memory:".to_string(),
///         }))
///     }
/// }
/// ```
///
/// Service with dependencies:
///
/// ```rust
/// use diode::{Service, AppBuilder, StdError, Dependencies, ServiceDependencyExt};
/// use std::sync::Arc;
///
/// struct ConfigService;
/// struct ApiService {
///     config: Arc<ConfigService>,
/// }
///
/// impl Service for ConfigService {
///     type Handle = Arc<Self>;
///     async fn build(_app: &AppBuilder) -> Result<Self::Handle, StdError> {
///         Ok(Arc::new(Self))
///     }
/// }
///
/// impl Service for ApiService {
///     type Handle = Arc<Self>;
///
///     async fn build(app: &AppBuilder) -> Result<Self::Handle, StdError> {
///         let config = app.get_component::<Arc<ConfigService>>()
///             .ok_or("ConfigService not found")?;
///
///         Ok(Arc::new(Self { config }))
///     }
///
///     fn dependencies() -> Dependencies {
///         Dependencies::new().service::<ConfigService>()
///     }
/// }
/// ```
pub trait Service: Send + Sync {
    /// The handle type that represents this service when injected into other components.
    ///
    /// This is typically `Arc<Self>` for services that need to be shared across multiple
    /// consumers, but can be any type that implements the required bounds.
    type Handle: Send + Sync + 'static;

    /// Builds an instance of this service asynchronously.
    ///
    /// This method is called during the application build process when the service
    /// is needed. It receives a reference to the application builder, which can be
    /// used to retrieve dependencies that have already been built.
    ///
    /// # Arguments
    ///
    /// * `app` - Reference to the application builder containing registered components.
    ///
    /// # Returns
    ///
    /// Returns `Ok(Self::Handle)` on successful service creation, or an error if
    /// the service cannot be built (e.g., missing dependencies, initialization failure).
    fn build(
        app: &AppBuilder,
    ) -> impl std::future::Future<Output = Result<Self::Handle, StdError>> + Send;

    /// Declares the dependencies this service requires.
    ///
    /// Dependencies are used to determine the initialization order of services.
    /// Services with dependencies will be built after their dependencies are available.
    ///
    /// # Returns
    ///
    /// Returns a `Dependencies` object describing required service or plugin dependencies.
    fn dependencies() -> Dependencies {
        Dependencies::new()
    }
}

/// Internal plugin that wraps a service to integrate it into the plugin system.
///
/// This struct is used internally by the framework to treat services as plugins,
/// enabling them to participate in the dependency resolution and initialization process.
struct ServiceProvider<T>(PhantomData<T>)
where
    T: Service;

impl<T> Plugin for ServiceProvider<T>
where
    T: Service,
{
    async fn build(&self, app: &mut AppBuilder) -> Result<(), StdError> {
        app.add_component(T::build(app).await?);
        Ok(())
    }

    fn dependencies(&self) -> Dependencies {
        T::dependencies()
    }
}

/// Extension trait for `AppBuilder` to add service registration methods.
///
/// This trait provides convenient methods for registering services with the application
/// builder. Services registered this way will be automatically built during the
/// application build process according to their declared dependencies.
pub trait AddServiceExt {
    /// Registers a service with the application builder.
    ///
    /// The service will be built during the application build process and its handle
    /// will be available for injection into other services or retrieval from the final app.
    ///
    /// # Type Parameters
    ///
    /// * `T` - The service type to register. Must implement `Service + 'static`.
    ///
    /// # Returns
    ///
    /// Returns `&mut Self` for method chaining.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use diode::{App, Service, AddServiceExt, StdError};
    /// use std::sync::Arc;
    ///
    /// struct MyService;
    ///
    /// impl Service for MyService {
    ///     type Handle = Arc<Self>;
    ///     async fn build(_app: &diode::AppBuilder) -> Result<Self::Handle, StdError> {
    ///         Ok(Arc::new(Self))
    ///     }
    /// }
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let app = App::builder()
    ///     .add_service::<MyService>()
    ///     .build()
    ///     .await?;
    /// # Ok(())
    /// # }
    /// ```
    fn add_service<T>(&mut self) -> &mut Self
    where
        T: Service + 'static;

    /// Checks if a service of the specified type has been registered.
    ///
    /// # Type Parameters
    ///
    /// * `T` - The service type to check for.
    ///
    /// # Returns
    ///
    /// Returns `true` if the service has been registered, `false` otherwise.
    fn has_service<T>(&self) -> bool
    where
        T: Service + 'static;
}

impl AddServiceExt for AppBuilder {
    fn add_service<T>(&mut self) -> &mut Self
    where
        T: Service + 'static,
    {
        self.add_plugin(ServiceProvider::<T>(PhantomData));
        self
    }

    fn has_service<T>(&self) -> bool
    where
        T: Service + 'static,
    {
        self.has_plugin::<ServiceProvider<T>>()
    }
}

/// Extension trait for `Dependencies` to add service dependency declarations.
///
/// This trait provides a convenient method for declaring dependencies on services
/// within a `Dependencies` object.
pub trait ServiceDependencyExt {
    /// Adds a service dependency to the dependencies set.
    ///
    /// This method allows declaring that a plugin or service depends on another service,
    /// ensuring the dependency will be built before the dependent component.
    ///
    /// # Type Parameters
    ///
    /// * `T` - The service type to depend on. Must implement `Service + 'static`.
    ///
    /// # Returns
    ///
    /// Returns `Self` for method chaining.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use diode::{Dependencies, Service, ServiceDependencyExt, StdError};
    /// use std::sync::Arc;
    ///
    /// struct DatabaseService;
    /// struct ApiService;
    ///
    /// impl Service for DatabaseService {
    ///     type Handle = Arc<Self>;
    ///     async fn build(_app: &diode::AppBuilder) -> Result<Self::Handle, StdError> {
    ///         Ok(Arc::new(Self))
    ///     }
    /// }
    ///
    /// impl Service for ApiService {
    ///     type Handle = Arc<Self>;
    ///     async fn build(_app: &diode::AppBuilder) -> Result<Self::Handle, StdError> {
    ///         Ok(Arc::new(Self))
    ///     }
    ///
    ///     fn dependencies() -> Dependencies {
    ///         Dependencies::new().service::<DatabaseService>()
    ///     }
    /// }
    /// ```
    fn service<T>(self) -> Self
    where
        T: Service + 'static;
}

impl ServiceDependencyExt for Dependencies {
    fn service<T>(self) -> Self
    where
        T: Service + 'static,
    {
        self.plugin::<ServiceProvider<T>>()
    }
}
