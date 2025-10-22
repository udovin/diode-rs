use std::collections::BTreeMap;
use std::path::PathBuf;

use diode::{Service, StdError};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::{Config, DynamicConfigService};

use super::DynamicConfigUpdater;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DynamicConfigFileConfig {
    pub path: PathBuf,
}

impl crate::ConfigSection for DynamicConfigFileConfig {
    fn key() -> &'static str {
        "dynamic_config_file"
    }
}

/// File-based dynamic configuration provider
#[derive(Service)]
pub struct DynamicConfigFile {
    #[inject(Config)]
    config: DynamicConfigFileConfig,
}

impl DynamicConfigService for DynamicConfigFile {
    async fn get_snapshot(&self) -> Result<BTreeMap<String, serde_json::Value>, StdError> {
        let path = &self.config.path;
        tracing::debug!(path = ?path, "Reading config file");
        let content = tokio::fs::read_to_string(&path).await.map_err(|e| {
            tracing::warn!(path = ?path, error = %e, "Failed to read config file");
            e
        })?;
        let config: BTreeMap<String, serde_json::Value> =
            serde_json::from_str(&content).map_err(|e| {
                tracing::warn!(path = ?path, error = %e, "Failed to parse config file");
                e
            })?;
        tracing::debug!(path = ?path, keys = config.len(), "Successfully loaded config file");
        Ok(config)
    }

    async fn watch_changes(
        &self,
        updater: DynamicConfigUpdater,
        shutdown: CancellationToken,
    ) -> Result<(), StdError> {
        let path = &self.config.path;
        tracing::info!(path = ?path, "Starting file watcher for dynamic config");
        let (tx, mut rx) = mpsc::channel(1);
        let mut watcher = RecommendedWatcher::new(
            move |res: Result<notify::Event, notify::Error>| {
                if let Err(e) = tx.try_send(res) {
                    tracing::warn!(error = %e, "Failed to send file watch event");
                }
            },
            notify::Config::default(),
        )
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to create file watcher");
            e
        })?;
        watcher
            .watch(path, RecursiveMode::NonRecursive)
            .map_err(|e| {
                tracing::error!(path = ?path, error = %e, "Failed to start watching file");
                e
            })?;
        match self.get_snapshot().await {
            Ok(snapshot) => {
                tracing::info!(path = ?path, "Loaded initial config snapshot");
                updater.set_snapshot(snapshot);
            }
            Err(e) => {
                tracing::error!(path = ?path, error = %e, "Failed to load initial config snapshot");
                return Err(e);
            }
        }
        loop {
            tokio::select! {
                event = rx.recv() => {
                    match event {
                        Some(Ok(event)) => {
                            tracing::debug!(path = ?path, event = ?event, "File watch event received");
                            if event.kind.is_modify() {
                                match self.get_snapshot().await {
                                    Ok(snapshot) => {
                                        tracing::info!(path = ?path, "Config file updated, reloading");
                                        updater.set_snapshot(snapshot);
                                    }
                                    Err(e) => {
                                        tracing::error!(path = ?path, error = %e, "Failed to reload config after file change");
                                    }
                                }
                            }
                        }
                        Some(Err(e)) => {
                            tracing::warn!(path = ?path, error = %e, "File watch error");
                        }
                        None => {
                            tracing::debug!("File watch channel closed");
                            break;
                        }
                    }
                }
                _ = shutdown.cancelled() => {
                    tracing::debug!(path = ?path, "File watcher shutting down");
                    break;
                }
            }
        }
        Ok(())
    }
}
