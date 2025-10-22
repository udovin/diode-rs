use tracing::Instrument;

use std::collections::BTreeMap;
use std::marker::PhantomData;
use std::path::PathBuf;
use std::sync::{
    Arc, RwLock,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use diode::{
    AddServiceExt, App, AppBuilder, Dependencies, Plugin, Service, ServiceDependencyExt, StdError,
};
use serde::{Deserialize, Deserializer, Serialize, de::DeserializeOwned};
use tokio_util::sync::CancellationToken;

use crate::{AddDaemonExt, Config, ConfigSection, Daemon, defer};

/// Configuration for dynamic config system
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct DynamicConfigConfig {
    /// Path to cache file for persistent storage
    #[serde(default)]
    pub cache_path: Option<PathBuf>,
    /// How often to write cache to disk (default: 1 second)
    #[serde(default, deserialize_with = "deserialize_duration_option")]
    pub cache_period: Option<Duration>,
    /// Path to fallback config file
    #[serde(default)]
    pub fallback_path: Option<PathBuf>,
}

impl ConfigSection for DynamicConfigConfig {
    fn key() -> &'static str {
        "dynamic_config"
    }
}

/// Main dynamic configuration store
pub struct DynamicConfig {
    /// Fallback values loaded from file
    fallback: BTreeMap<String, serde_json::Value>,
    /// In-memory cache of configuration values
    cache: RwLock<BTreeMap<String, serde_json::Value>>,
    /// Flag indicating cache needs to be written to disk
    cache_dirty: Arc<AtomicBool>,
    /// Event subscribers for configuration changes
    subscribers:
        RwLock<BTreeMap<String, Vec<Box<dyn Fn(Option<&serde_json::Value>) + Send + Sync>>>>,
}

impl DynamicConfig {
    /// Get configuration value by key
    pub fn get<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        tracing::trace!(key = key, "Getting dynamic config value");
        let cache = self.cache.read().unwrap();
        let value = cache.get(key).or_else(|| self.fallback.get(key));
        value.and_then(|v| {
            serde_json::from_value(v.clone())
                .map_err(|e| {
                    tracing::warn!(key = key, error = %e, "Failed to deserialize config value");
                    e
                })
                .ok()
        })
    }

    /// Subscribe to configuration changes for a specific key
    pub fn subscribe<T, F>(&self, key: &str, callback: F)
    where
        T: DeserializeOwned + 'static,
        F: Fn(Option<T>) + Send + Sync + 'static,
    {
        let key = key.to_string();
        tracing::debug!(key = key, "Subscribing to dynamic config changes");
        // Call callback immediately with current value
        let cache = self.cache.read().unwrap();
        let value = cache
            .get(&key)
            .or_else(|| self.fallback.get(&key))
            .and_then(|v| {
                serde_json::from_value(v.clone())
                    .map_err(|e| {
                        tracing::warn!(key = key, error = %e, "Failed to deserialize config value");
                        e
                    })
                    .ok()
            });
        callback(value);
        // Add to subscribers
        let wrapper = Box::new(move |value: Option<&serde_json::Value>| {
            let typed_value = value.and_then(|v| serde_json::from_value(v.clone()).ok());
            callback(typed_value);
        });
        let mut subscribers = self.subscribers.write().unwrap();
        subscribers.entry(key).or_default().push(wrapper);
    }

    /// Update configuration snapshot (internal method for providers)
    pub fn update_snapshot(&self, snapshot: BTreeMap<String, serde_json::Value>) {
        tracing::debug!("Updating dynamic config snapshot");
        let mut cache = self.cache.write().unwrap();
        let mut changed_keys = Vec::new();
        // Update existing keys and add new ones
        for (key, value) in &snapshot {
            match cache.get(key) {
                Some(v) if v == value => {
                    // No change, skip
                    continue;
                }
                _ => {
                    cache.insert(key.clone(), value.clone());
                    changed_keys.push(key.clone());
                }
            }
        }
        // Remove keys that are no longer in snapshot
        let keys_to_remove: Vec<String> = cache
            .keys()
            .filter(|key| !snapshot.contains_key(*key))
            .cloned()
            .collect();
        for key in keys_to_remove {
            cache.remove(&key);
            changed_keys.push(key);
        }
        drop(cache);
        if !changed_keys.is_empty() {
            self.cache_dirty.store(true, Ordering::Relaxed);
            self.notify_subscribers(changed_keys);
        }
    }

    /// Update single configuration key (internal method for providers)
    fn update_key(&self, key: String, value: serde_json::Value) {
        tracing::debug!(key = key, "Updating dynamic config key");
        let mut cache = self.cache.write().unwrap();
        let changed = match cache.get(&key) {
            Some(existing) => existing != &value,
            None => true,
        };
        if changed {
            cache.insert(key.clone(), value);
            drop(cache);
            self.cache_dirty.store(true, Ordering::Relaxed);
            self.notify_subscribers(vec![key]);
        }
    }

    /// Remove configuration key (internal method for providers)
    fn remove_key(&self, key: &str) {
        tracing::debug!(key = key, "Removing dynamic config key");
        let mut cache = self.cache.write().unwrap();
        if cache.remove(key).is_some() {
            drop(cache);
            self.cache_dirty.store(true, Ordering::Relaxed);
            self.notify_subscribers(vec![key.to_string()]);
        }
    }

    /// Notify subscribers about configuration changes
    fn notify_subscribers(&self, changed_keys: Vec<String>) {
        let cache = self.cache.read().unwrap();
        let subscribers = self.subscribers.read().unwrap();
        for key in changed_keys {
            if let Some(key_subscribers) = subscribers.get(&key) {
                let value = cache.get(&key).or_else(|| self.fallback.get(&key));

                for subscriber in key_subscribers {
                    subscriber(value);
                }
            }
        }
    }

    /// Save config cache to disk
    async fn save_cache(&self, cache_path: &PathBuf) -> Result<(), StdError> {
        let content = {
            let cache = self.cache.read().unwrap();
            serde_json::to_string_pretty(&*cache)?
        };
        tokio::fs::write(cache_path, content).await?;
        tracing::debug!("Saved dynamic config cache to disk");
        Ok(())
    }
}

/// Custom deserializer for optional Duration that supports string format like "1s", "100ms", etc.
fn deserialize_duration_option<'de, D>(deserializer: D) -> Result<Option<Duration>, D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de::Error;

    #[derive(Deserialize)]
    #[serde(untagged)]
    enum DurationValue {
        String(String),
        Number(u64),
    }

    let value: Option<DurationValue> = Option::deserialize(deserializer)?;

    match value {
        None => Ok(None),
        Some(DurationValue::String(s)) => duration_str::parse(&s)
            .map(Some)
            .map_err(|e| D::Error::custom(format!("Invalid duration format '{s}': {e}"))),
        Some(DurationValue::Number(n)) => Ok(Some(Duration::from_secs(n))),
    }
}

/// Trait for dynamic configuration providers
pub trait DynamicConfigService: Service<Handle = Arc<Self>> {
    /// Get current snapshot of all configuration values
    fn get_snapshot(
        &self,
    ) -> impl Future<Output = Result<BTreeMap<String, serde_json::Value>, StdError>> + Send;

    /// Start watching for configuration changes
    fn watch_changes(
        &self,
        updater: DynamicConfigUpdater,
        shutdown: CancellationToken,
    ) -> impl Future<Output = Result<(), StdError>> + Send {
        // Default implementation: no watching, just wait for shutdown
        let _ = updater;
        async move {
            shutdown.cancelled().await;
            Ok(())
        }
    }
}

/// Updater interface for providers to update configuration
pub struct DynamicConfigUpdater {
    config: Arc<DynamicConfig>,
}

impl DynamicConfigUpdater {
    /// Update entire configuration snapshot
    pub fn set_snapshot(&self, snapshot: BTreeMap<String, serde_json::Value>) {
        self.config.update_snapshot(snapshot);
    }

    /// Update single configuration key
    pub fn update_key(&self, key: String, value: serde_json::Value) {
        self.config.update_key(key, value);
    }

    /// Remove configuration key
    pub fn remove_key(&self, key: &str) {
        self.config.remove_key(key);
    }
}

struct DynamicConfigProvider<T>(PhantomData<T>);

impl<T> Plugin for DynamicConfigProvider<T>
where
    T: DynamicConfigService + 'static,
{
    /// Apply the dynamic config plugin to the app builder
    async fn build(&self, app: &mut AppBuilder) -> Result<(), StdError> {
        // Get plugin configuration
        let config = app
            .get_component_ref::<Config>()
            .unwrap()
            .get::<DynamicConfigConfig>("dynamic_config")
            .unwrap_or_default();
        // Get fallback config
        let fallback = match &config.fallback_path {
            Some(path) => load_dynamic_config(path).await?,
            None => BTreeMap::new(),
        };
        // Get config service
        let service = app.get_component::<T::Handle>();
        // Get cache config
        let cache = match &config.cache_path {
            Some(path) => match load_dynamic_config(path).await {
                Ok(v) => Some(v),
                Err(e) => {
                    tracing::warn!(error = %e, "Failed to load dynamic config cache");
                    None
                }
            },
            None => None,
        };
        let (cache, cache_dirty) = match cache {
            Some(v) => (v, false),
            None => (
                match service.as_ref() {
                    Some(v) => v.get_snapshot().await?,
                    None => fallback.clone(),
                },
                true,
            ),
        };
        // Create DynamicConfig instance synchronously
        let dynamic_config = Arc::new(DynamicConfig {
            fallback,
            cache: RwLock::new(cache),
            cache_dirty: Arc::new(AtomicBool::new(cache_dirty)),
            subscribers: Default::default(),
        });
        app.add_component(dynamic_config.clone())
            .add_daemon(DynamicConfigDaemon {
                dynamic_config,
                service,
                config,
            });
        Ok(())
    }

    fn dependencies(&self) -> Dependencies {
        Dependencies::new().service::<T>()
    }
}

/// Daemon that manages dynamic configuration lifecycle
struct DynamicConfigDaemon<T> {
    dynamic_config: Arc<DynamicConfig>,
    service: Option<Arc<T>>,
    config: DynamicConfigConfig,
}

impl<T> Daemon for DynamicConfigDaemon<T>
where
    T: DynamicConfigService + 'static,
{
    async fn run(&self, _app: &App, shutdown: CancellationToken) -> Result<(), StdError> {
        let dynamic_config = self.dynamic_config.clone();
        let service = self.service.clone();
        let config = self.config.clone();
        let span = tracing::info_span!("dynamic_config_daemon");
        tracing::info!(parent: &span, "Dynamic config daemon starting");
        defer! {
            tracing::info!(parent: &span, "Dynamic config daemon stopped");
        }
        // Initialize with provider snapshot if available
        if let Some(service) = &service {
            match service.get_snapshot().await {
                Ok(snapshot) => {
                    tracing::info!(parent: &span, "Loaded initial snapshot from provider");
                    dynamic_config.update_snapshot(snapshot);
                }
                Err(e) => {
                    tracing::warn!(parent: &span, error = %e, "Failed to get initial snapshot from provider");
                }
            }
            // Start service watcher in background task
            let updater = DynamicConfigUpdater {
                config: dynamic_config.clone(),
            };
            let shutdown = shutdown.clone();
            let service = service.clone();
            tokio::spawn(async move {
                let span = tracing::info_span!("dynamic_config_provider");
                tracing::info!(parent: &span, "Dynamic config provider starting");
                defer! {
                    tracing::info!(parent: &span, "Dynamic config provider stopped");
                }
                if let Err(e) = service
                    .watch_changes(updater, shutdown.clone())
                    .instrument(span.clone())
                    .await
                {
                    tracing::error!(parent: &span, error = %e, "Dynamic config provider failed");
                    shutdown.cancel();
                }
            });
        }
        // Cache persistence loop
        if let Some(cache_path) = &config.cache_path {
            let cache_period = config
                .cache_period
                .unwrap_or_else(|| Duration::from_secs(10));
            tracing::debug!(parent: &span, cache_period = ?cache_period, "Starting cache persistence loop");
            loop {
                tokio::select! {
                    _ = tokio::time::sleep(cache_period) => {
                        if dynamic_config.cache_dirty.compare_exchange(
                            true,
                            false,
                            std::sync::atomic::Ordering::Relaxed,
                            std::sync::atomic::Ordering::Relaxed
                        ).is_ok()
                            && let Err(e) = dynamic_config.save_cache(cache_path).await
                        {
                            tracing::warn!(parent: &span, error = %e, "Failed to save cache to disk");
                            dynamic_config.cache_dirty.store(true, std::sync::atomic::Ordering::Relaxed);
                        }
                    }
                    _ = shutdown.cancelled() => {
                        if dynamic_config.cache_dirty.load(std::sync::atomic::Ordering::Relaxed) {
                            if let Err(e) = dynamic_config.save_cache(cache_path).await {
                                tracing::warn!(parent: &span, error = %e, "Failed to save dynamic config cache during shutdown");
                            } else {
                                tracing::debug!(parent: &span, "Saved dynamic config cache during shutdown");
                            }
                        }
                        break;
                    }
                }
            }
        } else {
            shutdown.cancelled().await;
        }
        Ok(())
    }
}

pub trait AddDynamicConfigExt {
    fn add_dynamic_config<T>(&mut self) -> &mut Self
    where
        T: DynamicConfigService + 'static;

    fn has_dynamic_config<T>(&self) -> bool
    where
        T: DynamicConfigService + 'static;
}

impl AddDynamicConfigExt for AppBuilder {
    fn add_dynamic_config<T>(&mut self) -> &mut Self
    where
        T: DynamicConfigService + 'static,
    {
        if !self.has_service::<T>() {
            self.add_service::<T>();
        }
        self.add_plugin(DynamicConfigProvider::<T>(PhantomData));
        self
    }

    fn has_dynamic_config<T>(&self) -> bool
    where
        T: DynamicConfigService + 'static,
    {
        self.has_plugin::<DynamicConfigProvider<T>>()
    }
}

async fn load_dynamic_config(
    path: &PathBuf,
) -> Result<BTreeMap<String, serde_json::Value>, StdError> {
    let content = tokio::fs::read_to_string(path).await?;
    let config: BTreeMap<String, serde_json::Value> = serde_json::from_str(&content)?;
    Ok(config)
}
