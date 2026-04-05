use std::error::Error as _;
use std::{any::type_name, ops::DerefMut, sync::Arc};

use diode::{
    AddServiceExt as _, App, AppContext, AppError, Component, Dependencies, Extract, ExtractMut,
    ExtractRef, Plugin, Service, ServiceDependencyExt as _, StdError,
};

struct PluginA;

impl Plugin for PluginA {
    async fn build(&self, _ctx: &AppContext) -> Result<(), StdError> {
        Ok(())
    }
}

struct PluginB;

impl Plugin for PluginB {
    async fn build(&self, _ctx: &AppContext) -> Result<(), StdError> {
        Ok(())
    }

    fn dependencies(&self) -> Dependencies {
        Dependencies::new().plugin::<PluginA>()
    }
}

struct PluginC;

impl Plugin for PluginC {
    async fn build(&self, ctx: &AppContext) -> Result<(), StdError> {
        ctx.add_plugin(PluginA);
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
    async fn build(&self, _ctx: &AppContext) -> Result<(), StdError> {
        Ok(())
    }

    fn dependencies(&self) -> Dependencies {
        Dependencies::new().plugin::<CyclePluginC>()
    }
}

struct CyclePluginB;

impl Plugin for CyclePluginB {
    async fn build(&self, _ctx: &AppContext) -> Result<(), StdError> {
        Ok(())
    }

    fn dependencies(&self) -> Dependencies {
        Dependencies::new().plugin::<CyclePluginA>()
    }
}

struct CyclePluginC;

impl Plugin for CyclePluginC {
    async fn build(&self, _ctx: &AppContext) -> Result<(), StdError> {
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
        Err(AppError::CircularDependency { .. }),
    ));
}

#[tokio::test]
async fn test_plugins_missing() {
    assert!(matches!(
        App::builder().add_plugin(CyclePluginA).build().await,
        Err(AppError::MissingDependency { .. })
    ));
}

struct BadPlugin;

impl Plugin for BadPlugin {
    async fn build(&self, _ctx: &AppContext) -> Result<(), StdError> {
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

    async fn build(_ctx: &AppContext) -> Result<Arc<Self>, StdError> {
        Ok(Arc::new(Self {}))
    }
}

struct ServiceB {
    #[allow(unused)]
    service_a: Arc<ServiceA>,
}

impl Service for ServiceB {
    type Handle = Arc<Self>;

    async fn build(ctx: &AppContext) -> Result<Arc<Self>, StdError> {
        let service_a = ctx
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
        Err(AppError::MissingDependency { .. })
    ));
}

#[tokio::test]
async fn test_error_circular_dependency_message() {
    let result = App::builder()
        .add_plugin(CyclePluginA)
        .add_plugin(CyclePluginB)
        .add_plugin(CyclePluginC)
        .build()
        .await;
    let Err(err) = result else { panic!("expected error") };
    let msg = err.to_string();
    assert!(msg.starts_with("Circular dependency:"), "{msg}");
    assert!(msg.contains(" -> "), "{msg}");
}

#[tokio::test]
async fn test_error_missing_dependency_message() {
    let result = App::builder()
        .add_service::<ServiceB>()
        .build()
        .await;
    let Err(err) = result else { panic!("expected error") };
    let msg = err.to_string();
    assert!(msg.starts_with("Missing dependencies:"), "{msg}");
    assert!(msg.contains("requires:"), "{msg}");
}

#[tokio::test]
async fn test_error_plugin_error_message() {
    let result = App::builder().add_plugin(BadPlugin).build().await;
    let Err(err) = result else { panic!("expected error") };
    let msg = err.to_string();
    assert!(msg.starts_with("Plugin error:"), "{msg}");
    assert!(err.source().is_some());
}

#[test]
fn test_error_missing_component_message() {
    let err = AppError::MissingComponent("my_crate::Config");
    assert_eq!(err.to_string(), "Missing component: my_crate::Config");
    assert!(err.source().is_none());
}

#[test]
fn test_dependencies_default_and_merge() {
    let _ = Dependencies::default();
    let deps_a = Dependencies::new().plugin::<PluginA>();
    let deps_b = Dependencies::new().plugin::<PluginB>();
    let _ = deps_a.merge(deps_b);
}

#[tokio::test]
async fn test_component_extract() {
    let mut builder = App::builder();
    builder.add_component(42i32);
    let val: i32 = <Component as Extract<i32>>::extract(&builder).unwrap();
    assert_eq!(val, 42);
}

#[tokio::test]
async fn test_component_extract_missing() {
    let builder = App::builder();
    let err = <Component as Extract<i32>>::extract(&builder).err().expect("expected error");
    assert!(matches!(err, AppError::MissingComponent(_)));
    assert!(err.to_string().contains("i32"));
}

#[tokio::test]
async fn test_component_extract_ref() {
    let mut builder = App::builder();
    builder.add_component(42i32);
    let r = <Component as ExtractRef<i32>>::extract_ref(&builder).unwrap();
    assert_eq!(*r, 42);
}

#[tokio::test]
async fn test_component_extract_ref_missing() {
    let builder = App::builder();
    let err = <Component as ExtractRef<i32>>::extract_ref(&builder).err().expect("expected error");
    assert!(matches!(err, AppError::MissingComponent(_)));
}

#[tokio::test]
async fn test_component_extract_mut() {
    let mut builder = App::builder();
    builder.add_component(vec![1, 2, 3]);
    {
        let mut r = <Component as ExtractMut<Vec<i32>>>::extract_mut(&builder).unwrap();
        r.deref_mut().push(4);
    }
    let v = builder.get_component::<Vec<i32>>().unwrap();
    assert_eq!(v, vec![1, 2, 3, 4]);
}

#[tokio::test]
async fn test_component_extract_mut_missing() {
    let builder = App::builder();
    let err = <Component as ExtractMut<Vec<i32>>>::extract_mut(&builder).err().expect("expected error");
    assert!(matches!(err, AppError::MissingComponent(_)));
}

#[tokio::test]
async fn test_extract_ref_app_context() {
    let builder = App::builder();
    let r = <AppContext as ExtractRef<AppContext>>::extract_ref(&builder).unwrap();
    assert!(!r.has_component::<i32>());
}

#[tokio::test]
async fn test_service_extract_missing() {
    let builder = App::builder();
    let err = <ServiceA as Extract<Arc<ServiceA>>>::extract(&builder).err().expect("expected error");
    assert!(matches!(err, AppError::MissingComponent(_)));
}

#[tokio::test]
async fn test_service_extract_ref_missing() {
    let builder = App::builder();
    let err = <ServiceA as ExtractRef<Arc<ServiceA>>>::extract_ref(&builder).err().expect("expected error");
    assert!(matches!(err, AppError::MissingComponent(_)));
}

#[tokio::test]
async fn test_app_has_component() {
    let app = App::builder()
        .add_component(42i32)
        .build()
        .await
        .unwrap();
    assert!(app.has_component::<i32>());
    assert!(!app.has_component::<String>());
}

#[tokio::test]
async fn test_app_get_component_ref() {
    let app = App::builder()
        .add_component("hello".to_string())
        .build()
        .await
        .unwrap();
    let r = app.get_component_ref::<String>().unwrap();
    assert_eq!(r, "hello");
}
