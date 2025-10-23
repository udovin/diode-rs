use std::time::Duration;

use diode::{AppBuilder, StdError};
use duration_str::deserialize_option_duration;
use opentelemetry::KeyValue;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::metrics::{MeterProviderBuilder, PeriodicReader, SdkMeterProvider};
use opentelemetry_sdk::{Resource, runtime};
use serde::{Deserialize, Serialize};

use crate::{Config, ConfigSection};

pub struct Metrics {
    meter_provider: SdkMeterProvider,
}

impl Metrics {
    pub fn build(app: &mut AppBuilder) -> Result<(), StdError> {
        if app.has_component::<Self>() {
            return Ok(());
        }
        let config = match app
            .get_component_ref::<Config>()
            .unwrap()
            .get::<Option<MetricsConfig>>("metrics")?
        {
            Some(v) => v,
            None => return Ok(()),
        };
        let meter_provider = {
            if let Some(otlp_exporter) = config.otlp_exporter {
                let exporter = opentelemetry_otlp::MetricExporter::builder()
                    .with_tonic()
                    .with_endpoint(
                        otlp_exporter
                            .endpoint
                            .unwrap_or(DEFAULT_OTLP_EXPORTER_ENDPOINT.into()),
                    )
                    .with_timeout(
                        otlp_exporter
                            .timeout
                            .unwrap_or(DEFAULT_OTLP_EXPORTER_TIMEOUT),
                    )
                    .build()?;
                let reader = PeriodicReader::builder(exporter, runtime::Tokio)
                    .with_interval(
                        otlp_exporter
                            .interval
                            .unwrap_or(DEFAULT_OTLP_EXPORTER_INTERVAL),
                    )
                    .build();
                MeterProviderBuilder::default()
                    .with_resource(Resource::new(vec![KeyValue::new(
                        "service.name",
                        otlp_exporter.service_name.unwrap_or("unknown".into()),
                    )]))
                    .with_reader(reader)
                    .build()
            } else {
                MeterProviderBuilder::default().build()
            }
        };
        // Setup meter provider.
        opentelemetry::global::set_meter_provider(meter_provider.clone());
        // Add app components.
        app.add_component(Self { meter_provider });
        Ok(())
    }
}

impl Drop for Metrics {
    fn drop(&mut self) {
        if let Err(err) = self.meter_provider.shutdown() {
            tracing::error!("Cannot shutdown meter provider: {err}");
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct MetricsConfig {
    #[serde(default)]
    pub otlp_exporter: Option<MetricsOtlpExporterConfig>,
}

#[derive(Serialize, Deserialize)]
pub struct MetricsOtlpExporterConfig {
    #[serde(default)]
    pub service_name: Option<String>,
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(default, deserialize_with = "deserialize_option_duration")]
    pub timeout: Option<Duration>,
    #[serde(default, deserialize_with = "deserialize_option_duration")]
    pub interval: Option<Duration>,
}

impl ConfigSection for MetricsConfig {
    fn key() -> &'static str {
        "metrics"
    }
}

const DEFAULT_OTLP_EXPORTER_ENDPOINT: &str = "https://localhost:4317/v1/metrics";
const DEFAULT_OTLP_EXPORTER_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_OTLP_EXPORTER_INTERVAL: Duration = Duration::from_secs(10);
