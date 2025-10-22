use std::{any::type_name, sync::Arc};

use diode::{
    AddServiceExt as _, App, AppBuilder, AppError, Dependencies, Plugin, Service,
    ServiceDependencyExt as _, StdError,
};

struct PluginA;

impl Plugin for PluginA {
    async fn build(&self, _app: &mut AppBuilder) -> Result<(), StdError> {
        Ok(())
    }
}

struct PluginB;

impl Plugin for PluginB {
    async fn build(&self, _app: &mut AppBuilder) -> Result<(), StdError> {
        Ok(())
    }

    fn dependencies(&self) -> Dependencies {
        Dependencies::new().plugin::<PluginA>()
    }
}

struct PluginC;

impl Plugin for PluginC {
    async fn build(&self, app: &mut AppBuilder) -> Result<(), StdError> {
        app.add_plugin(PluginA);
        Ok(())
    }
}

#[tokio::test]
async fn test_plugins() {
    App::builder()
        .add_plugin(PluginB)
        .add_plugin(PluginC)
        .build()
        .await
        .unwrap();
}

#[tokio::test]
#[should_panic]
async fn test_plugins_duplicates() {
    App::builder()
        .add_plugin(PluginA)
        .add_plugin(PluginB)
        .add_plugin(PluginC)
        .build()
        .await
        .unwrap();
}

struct CyclePluginA;

impl Plugin for CyclePluginA {
    async fn build(&self, _app: &mut AppBuilder) -> Result<(), StdError> {
        Ok(())
    }

    fn dependencies(&self) -> Dependencies {
        Dependencies::new().plugin::<CyclePluginC>()
    }
}

struct CyclePluginB;

impl Plugin for CyclePluginB {
    async fn build(&self, _app: &mut AppBuilder) -> Result<(), StdError> {
        Ok(())
    }

    fn dependencies(&self) -> Dependencies {
        Dependencies::new().plugin::<CyclePluginA>()
    }
}

struct CyclePluginC;

impl Plugin for CyclePluginC {
    async fn build(&self, _app: &mut AppBuilder) -> Result<(), StdError> {
        Ok(())
    }

    fn dependencies(&self) -> Dependencies {
        Dependencies::new().plugin::<CyclePluginB>()
    }
}

#[tokio::test]
async fn test_plugins_circular() {
    assert!(matches!(
        App::builder()
            .add_plugin(CyclePluginA)
            .add_plugin(CyclePluginB)
            .add_plugin(CyclePluginC)
            .build()
            .await,
        Err(AppError::CircularDependency),
    ));
}

#[tokio::test]
async fn test_plugins_missing() {
    assert!(matches!(
        App::builder().add_plugin(CyclePluginA).build().await,
        Err(AppError::MissingDependency)
    ));
}

struct BadPlugin;

impl Plugin for BadPlugin {
    async fn build(&self, _app: &mut AppBuilder) -> Result<(), StdError> {
        Err("Bad plugin".into())
    }
}

#[tokio::test]
async fn test_plugins_bad() {
    assert!(matches!(
        App::builder().add_plugin(BadPlugin).build().await,
        Err(AppError::PluginError(_))
    ));
}

struct ServiceA {}

impl Service for ServiceA {
    type Handle = Arc<Self>;

    async fn build(_app: &AppBuilder) -> Result<Arc<Self>, StdError> {
        Ok(Arc::new(Self {}))
    }
}

struct ServiceB {
    #[allow(unused)]
    service_a: Arc<ServiceA>,
}

impl Service for ServiceB {
    type Handle = Arc<Self>;

    async fn build(app: &AppBuilder) -> Result<Arc<Self>, StdError> {
        let service_a = app
            .get_component()
            .ok_or(format!("Missing dependency: {}", type_name::<ServiceA>()))?;
        Ok(Arc::new(Self { service_a }))
    }

    fn dependencies() -> Dependencies {
        Dependencies::new().service::<ServiceA>()
    }
}

#[tokio::test]
async fn test_services() {
    App::builder()
        .add_service::<ServiceB>()
        .add_service::<ServiceA>()
        .build()
        .await
        .unwrap();
}

#[tokio::test]
async fn test_services_bad() {
    assert!(matches!(
        App::builder().add_service::<ServiceB>().build().await,
        Err(AppError::MissingDependency)
    ));
}
