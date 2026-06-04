use std::mem::replace;
use std::pin::Pin;
use std::sync::Arc;

use axum::response::Response;
use axum::{extract::Request, response::IntoResponse};
use diode::{AddServiceExt as _, AppBuilder, AppContext, Service};

/// The continuation passed to a [`Middleware`]: runs the rest of the chain (the
/// next middleware, or the route handler).
///
/// Call [`call`](Next::call) to forward the (possibly modified) request inward
/// and obtain the response. Not calling it short-circuits the chain: your
/// response is returned without running any inner middleware or the handler.
pub trait Next: Send + Sync {
    /// Forwards `request` to the rest of the chain and returns its response.
    fn call(self, request: Request) -> impl Future<Output = Response> + Send;
}

struct NextImpl<T>(T);

impl<T, R, E> Next for NextImpl<T>
where
    T: tower::Service<Request, Response = R, Error = E> + Send + Sync,
    T::Future: Send + 'static,
    R: IntoResponse,
    E: IntoResponse,
{
    async fn call(mut self, request: Request) -> Response {
        self.0.call(request).await.into_response()
    }
}

#[doc(hidden)]
pub struct MiddlewareLayerImpl<T>(pub Arc<T>);

impl<T> Clone for MiddlewareLayerImpl<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T, S> tower::Layer<S> for MiddlewareLayerImpl<T> {
    type Service = MiddlewareServiceImpl<T, S>;

    fn layer(&self, inner: S) -> Self::Service {
        Self::Service {
            middleware: self.0.clone(),
            inner,
        }
    }
}

#[doc(hidden)]
pub struct MiddlewareServiceImpl<T, S> {
    middleware: Arc<T>,
    inner: S,
}

impl<T, S> Clone for MiddlewareServiceImpl<T, S>
where
    S: Clone,
{
    fn clone(&self) -> Self {
        Self {
            middleware: self.middleware.clone(),
            inner: self.inner.clone(),
        }
    }
}

impl<T, S> tower::Service<Request> for MiddlewareServiceImpl<T, S>
where
    T: Middleware + 'static,
    S: tower::Service<Request> + Clone + Send + Sync + 'static,
    S::Response: IntoResponse,
    S::Error: IntoResponse,
    S::Future: Send + 'static,
    T::Error: IntoResponse,
{
    type Response = Response;
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
        let inner = replace(&mut self.inner, clone);
        let middleware = self.middleware.clone();
        let next = NextImpl(inner);
        Box::pin(async move {
            match middleware.call(request, next).await {
                Ok(response) => Ok(response.into_response()),
                Err(err) => Ok(err.into_response()),
            }
        })
    }
}

/// Middleware that wraps request handling on an HTTP server.
///
/// A middleware receives each request together with a [`Next`] continuation. It
/// may inspect or modify the request, short-circuit with its own response, or
/// call `next` and then inspect or modify the response.
///
/// Register it as a concrete instance with [`AddMiddlewareExt::add_middleware`],
/// or resolved from the DI container with
/// [`AddMiddlewareServiceExt::add_middleware_service`] (the latter additionally
/// requires the type to be a [`Service`], so it can hold injected dependencies).
/// Attach it to routes with the `#[router(middleware = [..])]` /
/// `#[route(middleware = [..])]` macro attributes.
///
/// # Ordering
///
/// Within a `middleware = [A, B]` list the first entry is outermost: it runs
/// first on the request and last on the response. Router-level middleware wraps
/// route-level middleware. So `#[router(middleware = [A, B])]` combined with
/// `#[route(middleware = [C, D])]` enters as `A, B, C, D` and unwinds as
/// `D, C, B, A`.
pub trait Middleware: Send + Sync {
    /// Error type rendered into a response when [`call`](Middleware::call)
    /// returns `Err`.
    type Error: IntoResponse;

    /// Handles `request`, optionally delegating to `next`.
    fn call(
        &self,
        request: Request,
        next: impl Next,
    ) -> impl Future<Output = Result<Response, Self::Error>> + Send;
}

/// Registers concrete [`Middleware`] instances so the router macros can resolve
/// them.
///
/// Lives on [`AppContext`], so middleware can be registered while configuring the
/// [`AppBuilder`] or from within a plugin's `build`. A middleware is identified by
/// its type and stored as an `Arc<T>` component; reference it from a router with
/// the `#[router(middleware = [..])]` / `#[route(middleware = [..])]` attributes.
pub trait AddMiddlewareExt {
    /// Registers `middleware` so routers can reference it by type `T`.
    ///
    /// # Panics
    ///
    /// Panics if a middleware of type `T` is already registered (as an instance,
    /// or - once the app is built - as a service). Guard with
    /// [`has_middleware`](AddMiddlewareExt::has_middleware) when this can happen.
    fn add_middleware<T>(&self, middleware: impl Into<Arc<T>>)
    where
        T: Middleware + 'static;

    /// Returns whether a middleware of type `T` is registered (its `Arc<T>` is
    /// present as a component).
    fn has_middleware<T>(&self) -> bool
    where
        T: Middleware + 'static;
}

impl AddMiddlewareExt for AppContext {
    fn add_middleware<T>(&self, middleware: impl Into<Arc<T>>)
    where
        T: Middleware + 'static,
    {
        self.add_component::<Arc<T>>(middleware.into());
    }

    fn has_middleware<T>(&self) -> bool
    where
        T: Middleware + 'static,
    {
        self.has_component::<Arc<T>>()
    }
}

/// Registers middleware resolved from the dependency-injection container.
///
/// The middleware type `T` is a [`Service`]; it is built by the container (with
/// its dependencies) and its handle - an `Arc<T>` - becomes the component the
/// router macros resolve. The service is added if it is not already present.
pub trait AddMiddlewareServiceExt {
    /// Registers the [`Service`] `T` so it is available as middleware.
    ///
    /// # Panics
    ///
    /// Building the [`App`](diode::App) panics if `T` is registered both as a
    /// service and as an instance via [`AddMiddlewareExt::add_middleware`].
    fn add_middleware_service<T>(&mut self) -> &mut Self
    where
        T: Middleware + Service<Handle = Arc<T>> + 'static;

    /// Returns whether `T` is registered as a service (and therefore available
    /// as middleware).
    fn has_middleware_service<T>(&self) -> bool
    where
        T: Middleware + Service<Handle = Arc<T>> + 'static;
}

impl AddMiddlewareServiceExt for AppBuilder {
    fn add_middleware_service<T>(&mut self) -> &mut Self
    where
        T: Middleware + Service<Handle = Arc<T>> + 'static,
    {
        if !self.has_service::<T>() {
            self.add_service::<T>();
        }
        self
    }

    fn has_middleware_service<T>(&self) -> bool
    where
        T: Middleware + Service<Handle = Arc<T>> + 'static,
    {
        self.has_service::<T>()
    }
}
