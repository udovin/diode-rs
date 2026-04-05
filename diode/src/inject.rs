use std::ops::{Deref, DerefMut};

use crate::{
    AppContext, AppError, ComponentMut, ComponentRef, Dependencies, Service, ServiceDependencyExt,
};

/// Trait for extracting owned values from the application context.
pub trait Extract<T> {
    fn extract(ctx: &AppContext) -> Result<T, AppError>;

    fn dependencies() -> Dependencies {
        Dependencies::new()
    }
}

/// Trait for extracting borrowed references from the application context.
///
/// The associated type `Ref` allows each implementation to choose its
/// return type — `ComponentRef<T>` for stored components, `&T` for
/// values available directly (e.g. `AppContext` itself).
pub trait ExtractRef<T> {
    type Ref<'a>: Deref<Target = T> + 'a;

    fn extract_ref(ctx: &AppContext) -> Result<Self::Ref<'_>, AppError>;

    fn dependencies() -> Dependencies {
        Dependencies::new()
    }
}

/// Trait for extracting mutable references from the application context.
///
/// Similar to [`ExtractRef`], but provides `DerefMut` access to the component.
///
/// # Deadlock warning
///
/// The returned guard holds a write lock on the underlying storage shard.
/// Combining a `&mut` inject parameter with other `&` or `&mut` inject
/// parameters in the same `#[factory]` method may deadlock if the components
/// are stored in the same shard. The `#[service]` macro rejects such
/// combinations at compile time.
///
/// If you need multiple references where at least one is mutable, inject
/// `&AppContext` and manage guards manually:
///
/// ```rust,ignore
/// #[factory]
/// fn new(#[inject(AppContext)] ctx: &AppContext) -> Arc<Self> {
///     {
///         let mut registry = ctx.get_component_mut::<Registry>().unwrap();
///         registry.register("my_service");
///     } // guard dropped before next acquisition
///     let config = ctx.get_component_ref::<Config>().unwrap();
///     Arc::new(Self { /* ... */ })
/// }
/// ```
pub trait ExtractMut<T>: ExtractRef<T> {
    type RefMut<'a>: DerefMut<Target = T> + 'a;

    fn extract_mut(ctx: &AppContext) -> Result<Self::RefMut<'_>, AppError>;
}

impl<T, S> Extract<T> for S
where
    T: Clone + Send + Sync + 'static,
    S: Service<Handle = T> + 'static,
{
    fn extract(ctx: &AppContext) -> Result<T, AppError> {
        ctx.get_component::<T>().ok_or(AppError::MissingComponent(std::any::type_name::<T>()))
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
    type Ref<'a> = ComponentRef<'a, T>;

    fn extract_ref(ctx: &AppContext) -> Result<Self::Ref<'_>, AppError> {
        ctx.get_component_ref::<T>().ok_or(AppError::MissingComponent(std::any::type_name::<T>()))
    }

    fn dependencies() -> Dependencies {
        Dependencies::new().service::<S>()
    }
}

impl ExtractRef<AppContext> for AppContext {
    type Ref<'a> = &'a AppContext;

    fn extract_ref(ctx: &AppContext) -> Result<&AppContext, AppError> {
        Ok(ctx)
    }
}

/// Generic extractor for any component stored in the application.
pub struct Component;

impl<T> Extract<T> for Component
where
    T: Clone + Send + Sync + 'static,
{
    fn extract(ctx: &AppContext) -> Result<T, AppError> {
        ctx.get_component::<T>().ok_or(AppError::MissingComponent(std::any::type_name::<T>()))
    }
}

impl<T> ExtractRef<T> for Component
where
    T: Send + Sync + 'static,
{
    type Ref<'a> = ComponentRef<'a, T>;

    fn extract_ref(ctx: &AppContext) -> Result<Self::Ref<'_>, AppError> {
        ctx.get_component_ref::<T>()
            .ok_or(AppError::MissingComponent(std::any::type_name::<T>()))
    }
}

impl<T> ExtractMut<T> for Component
where
    T: Send + Sync + 'static,
{
    type RefMut<'a> = ComponentMut<'a, T>;

    fn extract_mut(ctx: &AppContext) -> Result<Self::RefMut<'_>, AppError> {
        ctx.get_component_mut::<T>()
            .ok_or(AppError::MissingComponent(std::any::type_name::<T>()))
    }
}
