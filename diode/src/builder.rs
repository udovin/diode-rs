use std::ops::Deref;
use std::sync::Mutex;

use dashmap::DashMap;

use crate::{App, AppContext, AppError, Plugin};

/// Builder for constructing an [`App`] with a fluent API.
///
/// `AppBuilder` provides `&mut self` methods for chaining during the
/// configuration phase. It derefs to [`AppContext`], so all read methods
/// (`has_component`, `has_plugin`, etc.) are available without duplication.
///
/// # Examples
///
/// ```rust
/// use diode::{App, Service, StdError, AddServiceExt};
/// use std::sync::Arc;
///
/// struct DatabaseService;
///
/// impl Service for DatabaseService {
///     type Handle = Arc<Self>;
///     async fn build(_ctx: &diode::AppContext) -> Result<Self::Handle, StdError> {
///         Ok(Arc::new(Self))
///     }
/// }
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let app = App::builder()
///     .add_service::<DatabaseService>()
///     .add_component("config_value".to_string())
///     .build()
///     .await?;
/// # Ok(())
/// # }
/// ```
pub struct AppBuilder {
    pub(crate) context: AppContext,
}

impl Deref for AppBuilder {
    type Target = AppContext;

    fn deref(&self) -> &AppContext {
        &self.context
    }
}

impl AppBuilder {
    /// Adds a plugin to the application.
    ///
    /// # Panics
    ///
    /// Panics if a plugin of the same type has already been added.
    pub fn add_plugin<T>(&mut self, plugin: T) -> &mut Self
    where
        T: Plugin + 'static,
    {
        self.context.add_plugin(plugin);
        self
    }

    /// Adds a component to the application.
    ///
    /// # Panics
    ///
    /// Panics if a component of the same type has already been added.
    pub fn add_component<T>(&mut self, component: T) -> &mut Self
    where
        T: Send + Sync + 'static,
    {
        self.context.add_component(component);
        self
    }

    /// Builds all plugins in dependency order and returns the final [`App`].
    ///
    /// This drains the builder's internal state. The builder should not be
    /// used after calling `build`.
    pub async fn build(&mut self) -> Result<App, AppError> {
        let context = std::mem::replace(
            &mut self.context,
            AppContext {
                components: DashMap::new(),
                plugins: DashMap::new(),
                pending_plugins: Mutex::new(Vec::new()),
            },
        );
        context.build_app().await
    }
}
