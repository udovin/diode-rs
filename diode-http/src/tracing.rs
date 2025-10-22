use std::pin::Pin;
use std::sync::Arc;
use std::time::Instant;

use axum::extract::MatchedPath;
use diode::StdError;
use opentelemetry::propagation::TextMapPropagator;
use opentelemetry::{
    propagation::Extractor,
    trace::{Status, TraceContextExt as _},
};
use opentelemetry_sdk::propagation::TraceContextPropagator;
use tower::{Layer, Service};
use tracing::Instrument as _;
use tracing_opentelemetry::OpenTelemetrySpanExt as _;

use crate::{Request, Response};

#[derive(Clone, Copy)]
pub(crate) struct TracingLayer;

impl<S> Layer<S> for TracingLayer {
    type Service = TracingMiddleware<S>;

    fn layer(&self, inner: S) -> Self::Service {
        TracingMiddleware { inner }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct TracingMiddleware<S> {
    inner: S,
}

impl<S> TracingMiddleware<S>
where
    S: Service<Request, Response = Response> + Send + Clone + 'static,
{
    async fn request(request: Request, mut inner: S) -> Result<S::Response, S::Error> {
        let headers = request.headers();
        let propagator = TraceContextPropagator::new();
        let parent_context = propagator.extract(&HeaderExtractor(headers));
        let span = tracing::info_span!("request", trace_id = tracing::field::Empty);
        span.set_parent(parent_context);
        span.set_attribute(
            "otel.name",
            format!("{} {}", request.method(), request.uri()),
        );
        span.set_attribute("otel.kind", "server");
        span.set_attribute("http.method", request.method().to_string());
        span.set_attribute("http.target", request.uri().to_string());
        if let Some(path) = request.extensions().get::<MatchedPath>() {
            span.set_attribute("http.route", path.as_str().to_owned());
        }
        let trace_id = span.context().span().span_context().trace_id();
        span.record("trace_id", trace_id.to_string());
        tracing::info!(parent: &span, method = ?request.method(), uri = ?request.uri(), "Request");
        let now = Instant::now();
        let mut response = inner.call(request).instrument(span.clone()).await?;
        let latency = now.elapsed().as_micros();
        let status = response.status();
        span.set_attribute("http.status_code", status.as_u16() as i64);
        if let Some(error) = response.extensions().get::<Arc<StdError>>() {
            tracing::error!(parent: &span, error = ?error, "Response error");
        }
        if status.is_client_error() {
            tracing::warn!(parent: &span, latency, status = ?status.as_u16(), "Response");
        } else if status.is_server_error() {
            span.set_status(Status::error(status.to_string()));
            tracing::error!(parent: &span, latency, status = ?status.as_u16(), "Response");
        } else {
            tracing::info!(parent: &span, latency, status = ?status.as_u16(), "Response");
        }
        response
            .headers_mut()
            .insert("X-Trace-Id", trace_id.to_string().parse().unwrap());
        Ok(response)
    }
}

impl<S> Service<Request> for TracingMiddleware<S>
where
    S: Service<Request, Response = Response> + Send + Clone + 'static,
    S::Future: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future =
        Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send + 'static>>;

    fn poll_ready(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: Request) -> Self::Future {
        let clone = self.inner.clone();
        let inner = std::mem::replace(&mut self.inner, clone);
        Box::pin(async move { Self::request(request, inner).await })
    }
}

struct HeaderExtractor<'a>(pub &'a axum::http::HeaderMap);

impl Extractor for HeaderExtractor<'_> {
    /// Get a value for a key from the HeaderMap.  If the value is not valid ASCII, returns None.
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(|value| value.to_str().ok())
    }

    /// Collect all the keys from the HeaderMap.
    fn keys(&self) -> Vec<&str> {
        self.0
            .keys()
            .map(|value| value.as_str())
            .collect::<Vec<_>>()
    }
}
