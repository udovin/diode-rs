use std::any::{Any, TypeId, type_name};
use std::collections::{HashMap, HashSet};
use std::mem::take;
use std::ops::{Deref, DerefMut};
use std::sync::Mutex;

use dashmap::DashMap;
use dashmap::mapref::one::{MappedRef, MappedRefMut};

use crate::{AppError, DynPlugin, Plugin};

type ComponentBox = Box<dyn Any + Send + Sync>;

/// A smart pointer providing read access to a component stored in the application.
///
/// Implements `Deref<Target = T>`, allowing transparent access to the
/// underlying component. Returned by [`AppContext::get_component_ref`].
pub struct ComponentRef<'a, T>(MappedRef<'a, TypeId, ComponentBox, T>);

impl<T> Deref for ComponentRef<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

/// A smart pointer providing mutable access to a component stored in the application.
///
/// Implements `Deref<Target = T>` and `DerefMut`. Returned by
/// [`AppContext::get_component_mut`].
pub struct ComponentMut<'a, T>(MappedRefMut<'a, TypeId, ComponentBox, T>);

impl<T> Deref for ComponentMut<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T> DerefMut for ComponentMut<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.0
    }
}

/// Shared context available during the build phase.
///
/// `AppContext` is passed to [`Plugin::build`] and [`Service::build`]. All
/// methods take `&self` and use interior mutability, making it safe for
/// future parallel plugin execution.
///
/// Users do not construct `AppContext` directly. It is created internally
/// by [`AppBuilder::build`].
///
/// [`Service::build`]: crate::Service::build
/// [`AppBuilder::build`]: crate::AppBuilder::build
pub struct AppContext {
    pub(crate) components: DashMap<TypeId, ComponentBox>,
    pub(crate) plugins: DashMap<TypeId, Box<dyn DynPlugin>>,
    pub(crate) pending_plugins: Mutex<Vec<TypeId>>,
}

impl AppContext {
    /// Adds a component to the application.
    ///
    /// # Panics
    ///
    /// Panics if a component of the same type has already been added.
    ///
    /// # Deadlock
    ///
    /// This method briefly acquires a write lock on an internal storage shard.
    /// Calling it while a [`ComponentRef`] or [`ComponentMut`] is alive may
    /// deadlock if the new component maps to the same shard.
    pub fn add_component<T>(&self, component: T)
    where
        T: Send + Sync + 'static,
    {
        let type_id = TypeId::of::<T>();
        if self.components.contains_key(&type_id) {
            panic!("Component {} already added", type_name::<T>());
        }
        self.components.insert(type_id, Box::new(component));
    }

    /// Retrieves a component by type, returning a clone.
    pub fn get_component<T>(&self) -> Option<T>
    where
        T: Clone + Send + Sync + 'static,
    {
        self.get_component_ref::<T>().map(|r| r.clone())
    }

    /// Checks if a component of the specified type exists.
    pub fn has_component<T>(&self) -> bool
    where
        T: Send + Sync + 'static,
    {
        self.components.contains_key(&TypeId::of::<T>())
    }

    /// Retrieves a read reference to a component by type.
    pub fn get_component_ref<T>(&self) -> Option<ComponentRef<'_, T>>
    where
        T: Send + Sync + 'static,
    {
        self.components
            .get(&TypeId::of::<T>())
            .and_then(|r| r.try_map(|v| v.downcast_ref::<T>()).ok())
            .map(ComponentRef)
    }

    /// Retrieves a mutable reference to a component by type.
    ///
    /// # Deadlock
    ///
    /// The returned [`ComponentMut`] holds a write lock on an internal storage
    /// shard. Calling `get_component_ref`, `get_component_mut`, or
    /// `get_component` while a `ComponentMut` is alive will deadlock if the
    /// requested component happens to reside in the same shard. Always drop
    /// the `ComponentMut` before acquiring another guard:
    ///
    /// ```rust,ignore
    /// {
    ///     let mut registry = ctx.get_component_mut::<Registry>().unwrap();
    ///     registry.register("item");
    /// } // guard dropped
    /// let config = ctx.get_component_ref::<Config>().unwrap();
    /// ```
    pub fn get_component_mut<T>(&self) -> Option<ComponentMut<'_, T>>
    where
        T: Send + Sync + 'static,
    {
        self.components
            .get_mut(&TypeId::of::<T>())
            .and_then(|r| r.try_map(|v| v.downcast_mut::<T>()).ok())
            .map(ComponentMut)
    }

    /// Adds a plugin to the application.
    ///
    /// # Panics
    ///
    /// Panics if a plugin of the same type has already been added.
    pub fn add_plugin<T>(&self, plugin: T)
    where
        T: Plugin + 'static,
    {
        let type_id = TypeId::of::<T>();
        if self.plugins.contains_key(&type_id) {
            panic!("Plugin {} already added", plugin.name());
        }
        self.plugins.insert(type_id, Box::new(plugin));
        self.pending_plugins.lock().unwrap().push(type_id);
    }

    /// Checks if a plugin of the specified type has been added.
    pub fn has_plugin<T>(&self) -> bool
    where
        T: Plugin + 'static,
    {
        self.plugins.contains_key(&TypeId::of::<T>())
    }

    pub(crate) async fn build_app(self) -> Result<crate::App, AppError> {
        let mut graph = HashMap::new();
        let mut names: HashMap<TypeId, &'static str> = HashMap::new();
        let mut used = HashMap::new();
        loop {
            let pending_plugins = take(&mut *self.pending_plugins.lock().unwrap());
            if pending_plugins.is_empty() {
                break;
            }
            let mut order = Vec::new();
            for type_id in &pending_plugins {
                let plugin = self.plugins.get(type_id).unwrap();
                names.insert(*type_id, plugin.name());
                let deps = plugin.dependencies();
                names.extend(deps.names.iter());
                graph.insert(*type_id, deps.plugins);
            }
            let mut ready_plugins = HashSet::new();
            let mut deferred = Vec::new();
            let mut stack = Vec::new();
            for type_id in pending_plugins {
                if used.contains_key(&type_id) {
                    ready_plugins.insert(type_id);
                    continue;
                }
                if topological_sort(type_id, &graph, &names, &mut order, &mut used, &mut stack)? {
                    ready_plugins.insert(type_id);
                    continue;
                }
                deferred.push(type_id);
            }
            if order.is_empty() {
                let blocked = deferred
                    .iter()
                    .map(|type_id| {
                        let name = *names.get(type_id).unwrap_or(&"<unknown>");
                        let missing_deps: Vec<&'static str> = graph
                            .get(type_id)
                            .into_iter()
                            .flatten()
                            .filter(|dep_id| !used.contains_key(dep_id))
                            .map(|dep_id| *names.get(dep_id).unwrap_or(&"<unknown>"))
                            .collect();
                        (name, missing_deps)
                    })
                    .collect();
                return Err(AppError::MissingDependency { blocked });
            }
            for type_id in order {
                assert!(ready_plugins.remove(&type_id));
                let plugin = self.plugins.get(&type_id).unwrap();
                plugin.build(&self).await.map_err(AppError::PluginError)?;
            }
            assert!(ready_plugins.is_empty());
            self.pending_plugins.lock().unwrap().extend(deferred);
        }
        let components = self.components.into_iter().collect::<HashMap<_, _>>();
        Ok(crate::App { components })
    }
}

enum DependencyStatus {
    Pending,
    Ready,
}

fn topological_sort(
    type_id: TypeId,
    graph: &HashMap<TypeId, HashSet<TypeId>>,
    names: &HashMap<TypeId, &'static str>,
    order: &mut Vec<TypeId>,
    used: &mut HashMap<TypeId, DependencyStatus>,
    stack: &mut Vec<TypeId>,
) -> Result<bool, AppError> {
    let dependencies = match graph.get(&type_id) {
        Some(v) => v,
        None => return Ok(false),
    };
    stack.push(type_id);
    used.insert(type_id, DependencyStatus::Pending);
    for dep_type_id in dependencies {
        match used.get(dep_type_id) {
            Some(DependencyStatus::Pending) => {
                let cycle_start = stack.iter().position(|&id| id == *dep_type_id).unwrap();
                let cycle: Vec<&'static str> = stack[cycle_start..]
                    .iter()
                    .chain(std::iter::once(dep_type_id))
                    .map(|id| *names.get(id).unwrap_or(&"<unknown>"))
                    .collect();
                return Err(AppError::CircularDependency { cycle });
            }
            Some(DependencyStatus::Ready) => continue,
            None => {}
        }
        if !topological_sort(*dep_type_id, graph, names, order, used, stack)? {
            stack.pop();
            used.remove(&type_id);
            return Ok(false);
        }
    }
    stack.pop();
    used.insert(type_id, DependencyStatus::Ready);
    order.push(type_id);
    Ok(true)
}
