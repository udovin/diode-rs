use std::any::{Any, TypeId, type_name};
use std::collections::{HashMap, HashSet, hash_map};
use std::mem::take;

use async_trait::async_trait;

use crate::StdError;

/// Main application container that holds all registered components and services.
///
/// The `App` struct is the core of the dependency injection framework. It stores
/// type-safe components that can be retrieved by their type. Components are typically
/// service handles created during the application build process.
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
///
/// let service = app.get_component::<Arc<MyService>>().unwrap();
/// # Ok(())
/// # }
/// ```
pub struct App {
    components: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
}

/// Errors that can occur during application building or component retrieval.
#[derive(Debug)]
pub enum AppError {
    /// A circular dependency was detected between plugins or services.
    CircularDependency,
    /// A required dependency is missing from the application.
    MissingDependency,
    /// An error occurred within a plugin during initialization.
    PluginError(StdError),
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AppError::CircularDependency => write!(f, "Circular dependency detected"),
            AppError::MissingDependency => write!(f, "Missing dependency"),
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

impl App {
    /// Creates a new application builder for configuring and building an application.
    ///
    /// # Returns
    ///
    /// Returns an `AppBuilder` instance that can be used to register services,
    /// plugins, and components before building the final application.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use diode::App;
    ///
    /// let builder = App::builder();
    /// ```
    pub fn builder() -> AppBuilder {
        AppBuilder {
            components: HashMap::new(),
            plugins: HashMap::new(),
            pending_plugins: Vec::new(),
        }
    }

    /// Retrieves a component by type, returning a clone if the component implements `Clone`.
    ///
    /// # Type Parameters
    ///
    /// * `T` - The type of component to retrieve. Must implement `Clone + Send + Sync + 'static`.
    ///
    /// # Returns
    ///
    /// Returns `Some(T)` if the component exists and can be cloned, `None` otherwise.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use diode::App;
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let app = App::builder()
    ///     .add_component("Hello, World!".to_string())
    ///     .build()
    ///     .await?;
    ///
    /// let message = app.get_component::<String>().unwrap();
    /// assert_eq!(message, "Hello, World!");
    /// # Ok(())
    /// # }
    /// ```
    pub fn get_component<T>(&self) -> Option<T>
    where
        T: Clone + Send + Sync + 'static,
    {
        self.get_component_ref().cloned()
    }

    /// Checks if a component of the specified type exists in the application.
    ///
    /// # Type Parameters
    ///
    /// * `T` - The type of component to check for.
    ///
    /// # Returns
    ///
    /// Returns `true` if the component exists, `false` otherwise.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use diode::App;
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let app = App::builder()
    ///     .add_component(42i32)
    ///     .build()
    ///     .await?;
    ///
    /// assert!(app.has_component::<i32>());
    /// assert!(!app.has_component::<String>());
    /// # Ok(())
    /// # }
    /// ```
    pub fn has_component<T>(&self) -> bool
    where
        T: Send + Sync + 'static,
    {
        let type_id = TypeId::of::<T>();
        self.components.contains_key(&type_id)
    }

    /// Retrieves a reference to a component by type.
    ///
    /// # Type Parameters
    ///
    /// * `T` - The type of component to retrieve a reference to.
    ///
    /// # Returns
    ///
    /// Returns `Some(&T)` if the component exists, `None` otherwise.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use diode::App;
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let app = App::builder()
    ///     .add_component(vec![1, 2, 3])
    ///     .build()
    ///     .await?;
    ///
    /// let numbers = app.get_component_ref::<Vec<i32>>().unwrap();
    /// assert_eq!(numbers.len(), 3);
    /// # Ok(())
    /// # }
    /// ```
    pub fn get_component_ref<T>(&self) -> Option<&T>
    where
        T: Send + Sync + 'static,
    {
        let type_id = TypeId::of::<T>();
        self.components
            .get(&type_id)
            .and_then(|v| v.downcast_ref::<T>())
    }
}

/// Builder for constructing an `App` with registered services, plugins, and components.
///
/// The `AppBuilder` provides a fluent API for configuring an application before building it.
/// It handles dependency resolution, plugin initialization, and component registration in the
/// correct order based on declared dependencies.
///
/// # Examples
///
/// ```rust
/// use diode::{App, Service, StdError, AddServiceExt};
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
/// }
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let app = App::builder()
///     .add_service::<DatabaseService>()
///     .add_service::<ApiService>()
///     .add_component("config_value".to_string())
///     .build()
///     .await?;
/// # Ok(())
/// # }
/// ```
pub struct AppBuilder {
    components: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
    plugins: HashMap<TypeId, Box<dyn DynPlugin>>,
    pending_plugins: Vec<TypeId>,
}

impl AppBuilder {
    /// Adds a plugin to the application builder.
    ///
    /// Plugins are initialized during the build process in dependency order.
    /// Each plugin type can only be added once.
    ///
    /// # Type Parameters
    ///
    /// * `T` - The plugin type to add. Must implement `Plugin + 'static`.
    ///
    /// # Arguments
    ///
    /// * `plugin` - The plugin instance to add.
    ///
    /// # Returns
    ///
    /// Returns `&mut Self` for method chaining.
    ///
    /// # Panics
    ///
    /// Panics if a plugin of the same type has already been added.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use diode::{App, Plugin, Dependencies, StdError};
    ///
    /// struct MyPlugin;
    ///
    /// impl Plugin for MyPlugin {
    ///     async fn build(&self, _app: &mut diode::AppBuilder) -> Result<(), StdError> {
    ///         Ok(())
    ///     }
    /// }
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let app = App::builder()
    ///     .add_plugin(MyPlugin)
    ///     .build()
    ///     .await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn add_plugin<T>(&mut self, plugin: T) -> &mut Self
    where
        T: Plugin + 'static,
    {
        let type_id = TypeId::of::<T>();
        match self.plugins.entry(type_id) {
            hash_map::Entry::Occupied(_) => panic!("Plugin {} already added", plugin.name()),
            hash_map::Entry::Vacant(v) => {
                v.insert(Box::new(plugin));
                self.pending_plugins.push(type_id);
            }
        };
        self
    }

    /// Checks if a plugin of the specified type has been added.
    ///
    /// # Type Parameters
    ///
    /// * `T` - The plugin type to check for.
    ///
    /// # Returns
    ///
    /// Returns `true` if the plugin has been added, `false` otherwise.
    pub fn has_plugin<T>(&self) -> bool
    where
        T: Plugin + 'static,
    {
        let type_id = TypeId::of::<T>();
        self.plugins.contains_key(&type_id)
    }

    /// Adds a component directly to the application builder.
    ///
    /// Components added this way are immediately available and do not require
    /// dependency resolution. Each component type can only be added once.
    ///
    /// # Type Parameters
    ///
    /// * `T` - The component type to add. Must implement `Send + Sync + 'static`.
    ///
    /// # Arguments
    ///
    /// * `component` - The component instance to add.
    ///
    /// # Returns
    ///
    /// Returns `&mut Self` for method chaining.
    ///
    /// # Panics
    ///
    /// Panics if a component of the same type has already been added.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use diode::App;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let app = App::builder()
    ///     .add_component(42i32)
    ///     .add_component("configuration".to_string())
    ///     .build()
    ///     .await?;
    ///
    /// assert_eq!(app.get_component::<i32>(), Some(42));
    /// # Ok(())
    /// # }
    /// ```
    pub fn add_component<T>(&mut self, component: T) -> &mut Self
    where
        T: Send + Sync + 'static,
    {
        let type_id = TypeId::of::<T>();
        match self.components.entry(type_id) {
            hash_map::Entry::Occupied(_) => panic!("Component {} already added", type_name::<T>()),
            hash_map::Entry::Vacant(v) => {
                v.insert(Box::new(component));
            }
        };
        self
    }

    /// Retrieves a component by type, returning a clone if available.
    ///
    /// # Type Parameters
    ///
    /// * `T` - The type of component to retrieve.
    ///
    /// # Returns
    ///
    /// Returns `Some(T)` if the component exists and can be cloned, `None` otherwise.
    pub fn get_component<T>(&self) -> Option<T>
    where
        T: Clone + Send + Sync + 'static,
    {
        self.get_component_ref().cloned()
    }

    /// Checks if a component of the specified type exists.
    ///
    /// # Type Parameters
    ///
    /// * `T` - The type of component to check for.
    ///
    /// # Returns
    ///
    /// Returns `true` if the component exists, `false` otherwise.
    pub fn has_component<T>(&self) -> bool
    where
        T: Send + Sync + 'static,
    {
        let type_id = TypeId::of::<T>();
        self.components.contains_key(&type_id)
    }

    /// Retrieves a reference to a component by type.
    ///
    /// # Type Parameters
    ///
    /// * `T` - The type of component to retrieve a reference to.
    ///
    /// # Returns
    ///
    /// Returns `Some(&T)` if the component exists, `None` otherwise.
    pub fn get_component_ref<T>(&self) -> Option<&T>
    where
        T: Send + Sync + 'static,
    {
        let type_id = TypeId::of::<T>();
        self.components
            .get(&type_id)
            .and_then(|v| v.downcast_ref::<T>())
    }

    /// Retrieves a mutable reference to a component by type.
    ///
    /// # Type Parameters
    ///
    /// * `T` - The type of component to retrieve a mutable reference to.
    ///
    /// # Returns
    ///
    /// Returns `Some(&mut T)` if the component exists, `None` otherwise.
    pub fn get_component_mut<T>(&mut self) -> Option<&mut T>
    where
        T: Send + Sync + 'static,
    {
        let type_id = TypeId::of::<T>();
        self.components
            .get_mut(&type_id)
            .and_then(|v| v.downcast_mut::<T>())
    }

    pub async fn build(&mut self) -> Result<App, AppError> {
        let mut graph = HashMap::new();
        let mut used = HashMap::new();
        while !self.pending_plugins.is_empty() {
            let mut order = Vec::new();
            let pending_plugins = take(&mut self.pending_plugins);
            for type_id in &pending_plugins {
                let plugin = self.plugins.get(type_id).unwrap();
                graph.insert(*type_id, plugin.dependencies().plugins);
            }
            let mut ready_plugins = HashSet::new();
            for type_id in pending_plugins {
                if used.contains_key(&type_id) {
                    ready_plugins.insert(type_id);
                    continue;
                }
                if topological_sort(type_id, &graph, &mut order, &mut used)? {
                    ready_plugins.insert(type_id);
                    continue;
                }
                self.pending_plugins.push(type_id);
            }
            if order.is_empty() {
                return Err(AppError::MissingDependency);
            }
            for type_id in order {
                assert!(ready_plugins.remove(&type_id));
                // Safety: mutable AppBuilder never mutably references current plugin.
                let plugin = unsafe {
                    &*(self.plugins.get(&type_id).unwrap().as_ref() as *const dyn DynPlugin)
                };
                plugin.build(self).await.map_err(AppError::PluginError)?;
            }
            assert!(ready_plugins.is_empty());
        }
        // Drop plugins.
        take(&mut self.plugins);
        Ok(App {
            components: take(&mut self.components),
        })
    }
}

enum DependencyStatus {
    Pending,
    Ready,
}

fn topological_sort(
    type_id: TypeId,
    graph: &HashMap<TypeId, HashSet<TypeId>>,
    order: &mut Vec<TypeId>,
    used: &mut HashMap<TypeId, DependencyStatus>,
) -> Result<bool, AppError> {
    let dependencies = match graph.get(&type_id) {
        Some(v) => v,
        None => return Ok(false),
    };
    used.insert(type_id, DependencyStatus::Pending);
    for dep_type_id in dependencies {
        match used.get(dep_type_id) {
            Some(DependencyStatus::Pending) => return Err(AppError::CircularDependency),
            Some(DependencyStatus::Ready) => continue,
            None => {}
        }
        if !topological_sort(*dep_type_id, graph, order, used)? {
            used.remove(&type_id);
            return Ok(false);
        }
    }
    used.insert(type_id, DependencyStatus::Ready);
    order.push(type_id);
    Ok(true)
}

/// Represents dependencies between plugins and services in the application.
///
/// The `Dependencies` struct is used to declare what other plugins or services
/// a particular plugin or service depends on. This information is used during
/// the build process to ensure proper initialization order.
///
/// # Examples
///
/// ```rust
/// use diode::{Dependencies, Plugin, StdError};
///
/// struct DatabasePlugin;
/// struct ApiPlugin;
///
/// impl Plugin for DatabasePlugin {
///     async fn build(&self, _app: &mut diode::AppBuilder) -> Result<(), StdError> {
///         Ok(())
///     }
/// }
///
/// impl Plugin for ApiPlugin {
///     async fn build(&self, _app: &mut diode::AppBuilder) -> Result<(), StdError> {
///         Ok(())
///     }
///
///     fn dependencies(&self) -> Dependencies {
///         Dependencies::new().plugin::<DatabasePlugin>()
///     }
/// }
/// ```
#[derive(Clone)]
pub struct Dependencies {
    plugins: HashSet<TypeId>,
}

impl Dependencies {
    /// Creates a new empty dependencies set.
    ///
    /// # Returns
    ///
    /// Returns a new `Dependencies` instance with no dependencies.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use diode::Dependencies;
    ///
    /// let deps = Dependencies::new();
    /// ```
    pub fn new() -> Self {
        Self {
            plugins: HashSet::new(),
        }
    }

    /// Adds a plugin dependency to this dependencies set.
    ///
    /// # Type Parameters
    ///
    /// * `T` - The plugin type to depend on. Must implement `Plugin + 'static`.
    ///
    /// # Returns
    ///
    /// Returns `Self` for method chaining.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use diode::{Dependencies, Plugin, StdError};
    ///
    /// struct DatabasePlugin;
    ///
    /// impl Plugin for DatabasePlugin {
    ///     async fn build(&self, _app: &mut diode::AppBuilder) -> Result<(), StdError> {
    ///         Ok(())
    ///     }
    /// }
    ///
    /// let deps = Dependencies::new().plugin::<DatabasePlugin>();
    /// ```
    pub fn plugin<T>(mut self) -> Self
    where
        T: Plugin + 'static,
    {
        self.plugins.insert(TypeId::of::<T>());
        self
    }

    /// Merges another dependencies set into this one.
    ///
    /// # Arguments
    ///
    /// * `other` - Another `Dependencies` instance to merge.
    ///
    /// # Returns
    ///
    /// Returns `Self` with all dependencies from both sets.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use diode::Dependencies;
    ///
    /// let deps1 = Dependencies::new();
    /// let deps2 = Dependencies::new();
    /// let merged = deps1.merge(deps2);
    /// ```
    pub fn merge(mut self, other: Dependencies) -> Self {
        self.plugins.extend(other.plugins);
        self
    }
}

impl Default for Dependencies {
    fn default() -> Self {
        Self::new()
    }
}

pub trait Plugin: Send + Sync {
    fn build(&self, app: &mut AppBuilder) -> impl Future<Output = Result<(), StdError>> + Send;

    fn dependencies(&self) -> Dependencies {
        Dependencies::new()
    }
}

#[async_trait]
trait DynPlugin: Send + Sync {
    async fn build(&self, app: &mut AppBuilder) -> Result<(), StdError>;

    fn dependencies(&self) -> Dependencies {
        Dependencies::new()
    }

    fn name(&self) -> &'static str;
}

#[async_trait]
impl<T> DynPlugin for T
where
    T: Plugin,
{
    async fn build(&self, app: &mut AppBuilder) -> Result<(), StdError> {
        T::build(self, app).await
    }

    fn dependencies(&self) -> Dependencies {
        T::dependencies(self)
    }

    fn name(&self) -> &'static str {
        type_name::<T>()
    }
}
