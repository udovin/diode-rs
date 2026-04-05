use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::Mutex;

use dashmap::DashMap;

use crate::{AppBuilder, AppContext, StdError};

/// Main application container that holds all registered components and services.
///
/// `App` is immutable after construction. Components are retrieved by type.
///
/// # Examples
///
/// ```rust
/// use diode::{App, Service, StdError, AddServiceExt as _};
/// use std::sync::Arc;
///
/// struct MyService;
///
/// impl Service for MyService {
///     type Handle = Arc<Self>;
///
///     async fn build(_ctx: &diode::AppContext) -> Result<Self::Handle, StdError> {
///         Ok(Arc::new(Self))
///     }
/// }
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let app = App::builder()
///     .add_service::<MyService>()
///     .build()
///     .await?;
///
/// let service = app.get_component::<Arc<MyService>>().unwrap();
/// # Ok(())
/// # }
/// ```
pub struct App {
    pub(crate) components: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
}

impl App {
    /// Creates a new [`AppBuilder`] for configuring and building an application.
    pub fn builder() -> AppBuilder {
        AppBuilder {
            context: AppContext {
                components: DashMap::new(),
                plugins: DashMap::new(),
                pending_plugins: Mutex::new(Vec::new()),
            },
        }
    }

    /// Retrieves a component by type, returning a clone.
    pub fn get_component<T>(&self) -> Option<T>
    where
        T: Clone + Send + Sync + 'static,
    {
        self.get_component_ref().cloned()
    }

    /// Checks if a component of the specified type exists.
    pub fn has_component<T>(&self) -> bool
    where
        T: Send + Sync + 'static,
    {
        self.components.contains_key(&TypeId::of::<T>())
    }

    /// Retrieves a reference to a component by type.
    pub fn get_component_ref<T>(&self) -> Option<&T>
    where
        T: Send + Sync + 'static,
    {
        self.components
            .get(&TypeId::of::<T>())
            .and_then(|v| v.downcast_ref::<T>())
    }
}

/// Errors that can occur during application building.
#[derive(Debug)]
pub enum AppError {
    /// A circular dependency was detected between plugins or services.
    ///
    /// The `cycle` field contains the names of the plugins forming the cycle,
    /// e.g. `["A", "B", "A"]`.
    CircularDependency { cycle: Vec<&'static str> },
    /// A required dependency is missing from the application.
    ///
    /// Lists each blocked plugin together with the names of its unsatisfied
    /// dependencies.
    MissingDependency {
        blocked: Vec<(&'static str, Vec<&'static str>)>,
    },
    /// A component was not found during extraction.
    MissingComponent(&'static str),
    /// An error occurred within a plugin during initialization.
    PluginError(StdError),
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AppError::CircularDependency { cycle } => {
                write!(f, "Circular dependency: {}", cycle.join(" -> "))
            }
            AppError::MissingDependency { blocked } => {
                write!(f, "Missing dependencies:")?;
                for (plugin, deps) in blocked {
                    write!(f, "\n  {} requires: {}", plugin, deps.join(", "))?;
                }
                Ok(())
            }
            AppError::MissingComponent(name) => {
                write!(f, "Missing component: {name}")
            }
            AppError::PluginError(e) => write!(f, "Plugin error: {e}"),
        }
    }
}

impl std::error::Error for AppError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            AppError::PluginError(e) => Some(e.as_ref()),
            _ => None,
        }
    }
}

impl From<StdError> for AppError {
    fn from(value: StdError) -> Self {
        Self::PluginError(value)
    }
}
