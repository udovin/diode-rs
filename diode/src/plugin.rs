use std::any::{TypeId, type_name};
use std::collections::{HashMap, HashSet};

use async_trait::async_trait;

use crate::{AppContext, StdError};

/// Trait for modular components that participate in the build process.
///
/// Plugins receive an [`AppContext`] reference and can register components,
/// daemons, or other plugins.
pub trait Plugin: Send + Sync {
    fn build(&self, ctx: &AppContext) -> impl Future<Output = Result<(), StdError>> + Send;

    fn dependencies(&self) -> Dependencies {
        Dependencies::new()
    }
}

#[async_trait]
pub(crate) trait DynPlugin: Send + Sync {
    async fn build(&self, ctx: &AppContext) -> Result<(), StdError>;

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
    async fn build(&self, ctx: &AppContext) -> Result<(), StdError> {
        T::build(self, ctx).await
    }

    fn dependencies(&self) -> Dependencies {
        T::dependencies(self)
    }

    fn name(&self) -> &'static str {
        type_name::<T>()
    }
}

/// Declares dependencies between plugins and services.
///
/// Used during the build process to determine initialization order.
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
///     async fn build(&self, _ctx: &diode::AppContext) -> Result<(), StdError> {
///         Ok(())
///     }
/// }
///
/// impl Plugin for ApiPlugin {
///     async fn build(&self, _ctx: &diode::AppContext) -> Result<(), StdError> {
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
    pub(crate) plugins: HashSet<TypeId>,
    pub(crate) names: HashMap<TypeId, &'static str>,
}

impl Dependencies {
    /// Creates a new empty dependencies set.
    pub fn new() -> Self {
        Self {
            plugins: HashSet::new(),
            names: HashMap::new(),
        }
    }

    /// Adds a plugin dependency.
    pub fn plugin<T>(mut self) -> Self
    where
        T: Plugin + 'static,
    {
        let type_id = TypeId::of::<T>();
        self.plugins.insert(type_id);
        self.names.insert(type_id, type_name::<T>());
        self
    }

    /// Merges another dependencies set into this one.
    pub fn merge(mut self, other: Dependencies) -> Self {
        self.plugins.extend(other.plugins);
        self.names.extend(other.names);
        self
    }
}

impl Default for Dependencies {
    fn default() -> Self {
        Self::new()
    }
}
