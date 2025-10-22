use std::future::Future;
use std::pin::Pin;
use std::str::FromStr as _;
use std::sync::Arc;
use std::time::Duration;

use diode::{App, AppBuilder, StdError};
use duration_str::deserialize_option_duration;
use opentelemetry::trace::{SpanKind, TracerProvider as _};
use opentelemetry::{Key, KeyValue};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::export::trace::{ExportResult, SpanData, SpanExporter};
use opentelemetry_sdk::trace::TracerProvider;
use opentelemetry_sdk::{Resource, runtime};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use tracing_subscriber::filter::{Directive, EnvFilter};
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{Registry, reload};

use crate::{AddDaemonExt, CancellationToken, Config, ConfigSection, Daemon, DynamicConfig};

pub struct Tracing {
    default_level: tracing::Level,
    directives: Vec<Directive>,
    reload_handle: reload::Handle<EnvFilter, Registry>,
    tracer_provider: TracerProvider,
}

impl Tracing {
    pub fn build(app: &mut AppBuilder) -> Result<(), StdError> {
        if app.has_component::<Tracing>() {
            // Ensure that the tracing daemon is added.
            if !app.has_daemon::<TracingDaemon>() {
                app.add_daemon(TracingDaemon);
            }
            return Ok(());
        }
        let config = match app
            .get_component_ref::<Config>()
            .unwrap()
            .get::<Option<TracingConfig>>("tracing")
            .unwrap()
        {
            Some(v) => v,
            None => return Ok(()),
        };
        let mut directives = Vec::new();
        for directive in config.directives {
            directives.push(directive.parse().map_err(Box::new)?);
        }
        // Setup dynamic config level filter.
        let (env_filter, reload_handle) =
            reload::Layer::new(new_env_filter(&directives, config.level));
        // Setup OpenTelemetry tracer.
        let tracer_provider = {
            if let Some(otlp_exporter) = config.otlp_exporter {
                let exporter_builder = opentelemetry_otlp::SpanExporter::builder()
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
                    );
                let exporter = CustomSpanExporter::new(exporter_builder.build().unwrap());
                TracerProvider::builder()
                    .with_resource(Resource::new(vec![KeyValue::new(
                        "service.name",
                        otlp_exporter.service_name.unwrap_or("unknown".into()),
                    )]))
                    .with_batch_exporter(exporter, runtime::Tokio)
                    .build()
            } else {
                TracerProvider::builder().build()
            }
        };
        // Setup tracing registry.
        tracing_subscriber::registry()
            .with(env_filter)
            .with(tracing_subscriber::fmt::Layer::default())
            .with(tracing_opentelemetry::layer().with_tracer(tracer_provider.tracer("")))
            .init();
        // Add app components.
        app.add_component(Self {
            default_level: config.level,
            directives,
            reload_handle,
            tracer_provider,
        });
        app.add_daemon(TracingDaemon);
        Ok(())
    }
}

impl Drop for Tracing {
    fn drop(&mut self) {
        if let Err(err) = self.tracer_provider.shutdown() {
            tracing::error!("Cannot shutdown tracer provider: {err}");
        }
    }
}

struct TracingDaemon;

const TRACING_LEVEL_CONFIG_KEY: &str = "tracing_level";

impl Daemon for TracingDaemon {
    async fn run(&self, app: &App, shutdown: CancellationToken) -> Result<(), StdError> {
        let tracing = app.get_component_ref::<Tracing>().unwrap();
        let default_level = tracing.default_level;
        let directives = tracing.directives.clone();
        let reload_handle = tracing.reload_handle.clone();
        if let Some(dynamic_config) = app.get_component::<Arc<DynamicConfig>>() {
            dynamic_config.subscribe(TRACING_LEVEL_CONFIG_KEY, move |level: Option<String>| {
                let level = match level {
                    Some(v) => match tracing::Level::from_str(&v) {
                        Ok(v) => v,
                        Err(err) => {
                            tracing::error!("Cannot parse tracing level: {}", err);
                            return;
                        }
                    },
                    None => default_level,
                };
                reload_handle
                    .reload(new_env_filter(&directives, level))
                    .unwrap();
            });
        }
        shutdown.cancelled_owned().await;
        Ok(())
    }
}

impl Default for TracingConfig {
    fn default() -> Self {
        Self {
            level: default_level(),
            directives: Default::default(),
            otlp_exporter: None,
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct OtlpExporterConfig {
    #[serde(default)]
    pub service_name: Option<String>,
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(default, deserialize_with = "deserialize_option_duration")]
    pub timeout: Option<Duration>,
}

#[derive(Serialize, Deserialize)]
pub struct TracingConfig {
    #[serde(
        serialize_with = "serialize_level",
        deserialize_with = "deserialize_level",
        default = "default_level"
    )]
    pub level: tracing::Level,
    #[serde(default)]
    pub directives: Vec<String>,
    #[serde(default)]
    pub otlp_exporter: Option<OtlpExporterConfig>,
}

impl ConfigSection for TracingConfig {
    fn key() -> &'static str {
        "tracing"
    }
}

fn new_env_filter(directives: &Vec<Directive>, level: tracing::Level) -> EnvFilter {
    let mut filter = EnvFilter::default();
    for directive in directives {
        filter = filter.add_directive(directive.clone());
    }
    filter.add_directive(level.into())
}

const DEFAULT_OTLP_EXPORTER_ENDPOINT: &str = "https://localhost:4317/v1/traces";
const DEFAULT_OTLP_EXPORTER_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug)]
struct CustomSpanExporter<T> {
    inner: T,
}

impl<T> CustomSpanExporter<T> {
    pub fn new(inner: T) -> Self {
        Self { inner }
    }
}

impl<T> SpanExporter for CustomSpanExporter<T>
where
    T: SpanExporter,
{
    fn export(
        &mut self,
        mut batch: Vec<SpanData>,
    ) -> Pin<Box<dyn Future<Output = ExportResult> + Send + 'static>> {
        const OTEL_NAME_KEY: Key = Key::from_static_str("otel.name");
        const OTEL_KIND_KEY: Key = Key::from_static_str("otel.kind");
        const TRACE_ID_KEY: Key = Key::from_static_str("trace_id");
        for span in batch.iter_mut() {
            let mut otel_name = None;
            let mut otel_kind = None;
            span.attributes.retain(|v| {
                if v.key == OTEL_NAME_KEY {
                    otel_name = Some(v.value.clone());
                    false
                } else if v.key == OTEL_KIND_KEY {
                    otel_kind = Some(v.value.clone());
                    false
                } else if v.key == TRACE_ID_KEY {
                    false
                } else {
                    true
                }
            });
            if let Some(v) = otel_name {
                span.name = v.to_string().into();
            }
            if let Some(v) = otel_kind {
                match v.as_str().as_ref() {
                    "server" => span.span_kind = SpanKind::Server,
                    "client" => span.span_kind = SpanKind::Client,
                    "consumer" => span.span_kind = SpanKind::Consumer,
                    "producer" => span.span_kind = SpanKind::Producer,
                    _ => {}
                }
            }
        }
        self.inner.export(batch)
    }

    fn shutdown(&mut self) {
        self.inner.shutdown();
    }

    fn force_flush(&mut self) -> Pin<Box<dyn Future<Output = ExportResult> + Send + 'static>> {
        self.inner.force_flush()
    }

    fn set_resource(&mut self, resource: &opentelemetry_sdk::Resource) {
        self.inner.set_resource(resource);
    }
}

fn serialize_level<S>(v: &tracing::Level, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(v.as_str())
}

fn deserialize_level<'de, D>(deserializer: D) -> Result<tracing::Level, D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de::Error;
    String::deserialize(deserializer)
        .and_then(|v| tracing::Level::from_str(&v).map_err(|v| Error::custom(format!("{v}"))))
}

fn default_level() -> tracing::Level {
    tracing::Level::DEBUG
}
