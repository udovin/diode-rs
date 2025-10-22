use std::mem::replace;
use std::pin::Pin;
use std::{marker::PhantomData, sync::Arc};

use axum::response::Response;
use axum::{extract::Request, response::IntoResponse};
use diode::{
    AddServiceExt as _, AppBuilder, Dependencies, Plugin, Service, ServiceDependencyExt as _,
};

pub trait Next: Send + Sync {
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

pub trait MiddlewareService: Service<Handle = Arc<Self>> {
    type Error: IntoResponse;

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
    async fn build(&self, _app: &mut AppBuilder) -> Result<(), diode::StdError> {
        Ok(())
    }

    fn dependencies(&self) -> diode::Dependencies {
        Dependencies::new().service::<T>()
    }
}

pub trait AddMiddlewareExt {
    fn add_middleware<T>(&mut self) -> &mut Self
    where
        T: MiddlewareService + 'static;

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

pub trait MiddlewareDependencyExt {
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
