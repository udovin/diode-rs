use std::mem::replace;
use std::pin::Pin;
use std::{marker::PhantomData, sync::Arc};

use axum::response::Response;
use axum::{extract::Request, response::IntoResponse};
use diode::{
    AddServiceExt as _, AppBuilder, AppContext, Dependencies, Plugin, Service,
    ServiceDependencyExt as _,
};

/// The continuation passed to a [`MiddlewareService`]: runs the rest of the
/// chain (the next middleware, or the route handler).
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
            service: self.0.clone(),
            inner,
        }
    }
}

#[doc(hidden)]
pub struct MiddlewareServiceImpl<T, S> {
    service: Arc<T>,
    inner: S,
}

impl<T, S> Clone for MiddlewareServiceImpl<T, S>
where
    S: Clone,
{
    fn clone(&self) -> Self {
        Self {
            service: self.service.clone(),
            inner: self.inner.clone(),
        }
    }
}

impl<T, S> tower::Service<Request> for MiddlewareServiceImpl<T, S>
where
    T: MiddlewareService + 'static,
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
        let layer = self.service.clone();
        let next = NextImpl(inner);
        Box::pin(async move {
            match layer.call(request, next).await {
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
/// Middleware is a [`Service`], so it is built by the container and may hold
/// injected dependencies. Attach it to routes with the
/// `#[router(middleware = [..])]` / `#[route(middleware = [..])]` macro
/// attributes, and register the service with
/// [`AddMiddlewareExt::add_middleware`].
///
/// # Ordering
///
/// Within a `middleware = [A, B]` list the first entry is outermost: it runs
/// first on the request and last on the response. Router-level middleware wraps
/// route-level middleware. So `#[router(middleware = [A, B])]` combined with
/// `#[route(middleware = [C, D])]` enters as `A, B, C, D` and unwinds as
/// `D, C, B, A`.
pub trait MiddlewareService: Service<Handle = Arc<Self>> {
    /// Error type rendered into a response when
    /// [`call`](MiddlewareService::call) returns `Err`.
    type Error: IntoResponse;

    /// Handles `request`, optionally delegating to `next`.
    fn call(
        &self,
        request: Request,
        next: impl Next,
    ) -> impl Future<Output = Result<Response, Self::Error>> + Send;
}

struct MiddlewareProvider<T>(PhantomData<T>)
where
    T: MiddlewareService;

impl<T> Plugin for MiddlewareProvider<T>
where
    T: MiddlewareService + 'static,
{
    async fn build(&self, _ctx: &AppContext) -> Result<(), diode::StdError> {
        Ok(())
    }

    fn dependencies(&self) -> diode::Dependencies {
        Dependencies::new().service::<T>()
    }
}

/// Registers [`MiddlewareService`] implementations so the router macros can
/// resolve them.
///
/// A middleware must be registered here for any router that references it via
/// `#[router(middleware = [..])]` / `#[route(middleware = [..])]`; the generated
/// [`RouterBuilder`](crate::RouterBuilder) looks up the middleware's handle while
/// it builds.
pub trait AddMiddlewareExt {
    /// Registers the middleware service `T`.
    ///
    /// # Panics
    ///
    /// Panics if `T` is already registered as middleware. Guard with
    /// [`has_middleware`](AddMiddlewareExt::has_middleware) when this can happen.
    fn add_middleware<T>(&mut self) -> &mut Self
    where
        T: MiddlewareService + 'static;

    /// Returns whether `T` is registered as middleware.
    fn has_middleware<T>(&self) -> bool
    where
        T: MiddlewareService + 'static;
}

impl AddMiddlewareExt for AppBuilder {
    fn add_middleware<T>(&mut self) -> &mut Self
    where
        T: MiddlewareService + 'static,
    {
        if !self.has_service::<T>() {
            self.add_service::<T>();
        }
        self.add_plugin(MiddlewareProvider::<T>(PhantomData));
        self
    }

    fn has_middleware<T>(&self) -> bool
    where
        T: MiddlewareService + 'static,
    {
        self.has_plugin::<MiddlewareProvider<T>>()
    }
}

/// Declares a [`MiddlewareService`] as a [`Dependencies`] entry, so the plugin
/// or service declaring it is built only after the middleware is registered.
pub trait MiddlewareDependencyExt {
    /// Adds `T` as a middleware dependency.
    fn middleware<T>(self) -> Self
    where
        T: MiddlewareService + 'static;
}

impl MiddlewareDependencyExt for Dependencies {
    fn middleware<T>(self) -> Self
    where
        T: MiddlewareService + 'static,
    {
        self.plugin::<MiddlewareProvider<T>>()
    }
}
