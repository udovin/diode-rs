use std::marker::PhantomData;

use crate::{AppBuilder, AppContext, Dependencies, Plugin};

/// Type alias for boxed errors that can be sent across threads.
pub type StdError = Box<dyn std::error::Error + Send + Sync>;

/// Trait for defining injectable services with async initialization.
///
/// Services define how to build themselves, what dependencies they require,
/// and what handle type they expose via the DI container.
///
/// # Examples
///
/// ```rust
/// use diode::{Service, AppContext, StdError};
/// use std::sync::Arc;
///
/// struct DatabaseService {
///     connection_pool: String,
/// }
///
/// impl Service for DatabaseService {
///     type Handle = Arc<Self>;
///
///     async fn build(_ctx: &AppContext) -> Result<Self::Handle, StdError> {
///         Ok(Arc::new(Self {
///             connection_pool: "sqlite::memory:".to_string(),
///         }))
///     }
/// }
/// ```
pub trait Service: Send + Sync {
    /// The handle type exposed when this service is injected.
    type Handle: Send + Sync + 'static;

    /// Builds an instance of this service asynchronously.
    fn build(
        ctx: &AppContext,
    ) -> impl std::future::Future<Output = Result<Self::Handle, StdError>> + Send;

    /// Declares the dependencies this service requires.
    fn dependencies() -> Dependencies {
        Dependencies::new()
    }
}

/// Internal plugin that wraps a service into the plugin system.
struct ServiceProvider<T>(PhantomData<T>)
where
    T: Service;

impl<T> Plugin for ServiceProvider<T>
where
    T: Service,
{
    async fn build(&self, ctx: &AppContext) -> Result<(), StdError> {
        ctx.add_component(T::build(ctx).await?);
        Ok(())
    }

    fn dependencies(&self) -> Dependencies {
        T::dependencies()
    }
}

/// Extension trait for registering services on [`AppBuilder`].
pub trait AddServiceExt {
    fn add_service<T>(&mut self) -> &mut Self
    where
        T: Service + 'static;

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

/// Extension trait for declaring service dependencies.
pub trait ServiceDependencyExt {
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
