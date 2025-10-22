use diode::{AddServiceExt as _, App, AppBuilder, Component, Service, StdError, service};
use std::sync::Arc;

#[derive(Clone, Default)]
struct Config {
    valid: bool,
}

#[derive(Clone)]
struct SimpleService;

impl Service for SimpleService {
    type Handle = Self;

    async fn build(_app: &AppBuilder) -> Result<Self::Handle, StdError> {
        Ok(Self)
    }
}

#[derive(Service)]
struct ServiceWithDependency {
    #[allow(unused)]
    #[inject(SimpleService)]
    simple: SimpleService,
}

struct ServiceWithFactory {
    #[allow(unused)]
    simple: SimpleService,
}

#[service]
impl ServiceWithFactory {
    #[factory]
    fn new(
        #[inject(SimpleService)] simple: SimpleService,
        #[inject(Component)] config: &Config,
    ) -> Arc<Self> {
        assert!(config.valid);
        Arc::new(Self { simple })
    }
}

#[derive(Service)]
struct ServiceWithMultipleDependencies {
    #[allow(unused)]
    #[inject(SimpleService)]
    simple: SimpleService,
    #[allow(unused)]
    dependency: Arc<ServiceWithFactory>,
}

struct CustomService;

impl Service for CustomService {
    type Handle = Box<str>;

    async fn build(_app: &AppBuilder) -> Result<Self::Handle, StdError> {
        Ok("CustomService".into())
    }
}

struct ServiceWithFactory2;

#[service]
impl ServiceWithFactory2 {
    #[factory]
    fn new(
        #[inject(SimpleService)] _simple: SimpleService,
        _dependency: Arc<ServiceWithFactory>,
        #[inject(Component)] config: Config,
        #[inject(CustomService)] custom: Box<str>,
    ) -> Arc<Self> {
        assert!(config.valid);
        assert_eq!(custom.as_ref(), "CustomService");
        Arc::new(Self)
    }
}

struct AsyncFactory;

#[service]
impl AsyncFactory {
    #[factory]
    async fn new() -> Arc<Self> {
        Arc::new(Self)
    }
}

struct AsyncFactoryWithResult;

#[service]
impl AsyncFactoryWithResult {
    #[factory]
    async fn new() -> Result<Arc<Self>, std::io::Error> {
        Ok(Arc::new(Self))
    }
}

#[derive(Clone)]
struct SimpleFactory;

#[service]
impl SimpleFactory {
    #[factory]
    fn new() -> Arc<Self> {
        Arc::new(Self)
    }
}

struct FactoryWithCustomHandle;

#[service]
impl FactoryWithCustomHandle {
    #[factory]
    fn new() -> i32 {
        42
    }
}

struct FactoryWithResultAndDeps {
    #[allow(unused)]
    dep: SimpleService,
}

#[service]
impl FactoryWithResultAndDeps {
    #[factory]
    fn new(
        #[inject(SimpleService)] dep: SimpleService,
    ) -> Result<Arc<Self>, std::convert::Infallible> {
        Ok(Arc::new(Self { dep }))
    }
}

#[tokio::test]
async fn test_app_macros() {
    App::builder()
        .add_component(Config { valid: true })
        .add_service::<SimpleService>()
        .add_service::<ServiceWithDependency>()
        .add_service::<ServiceWithFactory>()
        .add_service::<ServiceWithMultipleDependencies>()
        .add_service::<ServiceWithFactory2>()
        .add_service::<CustomService>()
        .add_service::<AsyncFactory>()
        .add_service::<AsyncFactoryWithResult>()
        .add_service::<SimpleFactory>()
        .add_service::<FactoryWithCustomHandle>()
        .add_service::<FactoryWithResultAndDeps>()
        .build()
        .await
        .unwrap();
}

#[tokio::test]
async fn test_async_factory() {
    let app = App::builder()
        .add_service::<AsyncFactory>()
        .build()
        .await
        .unwrap();

    let service = app.get_component::<Arc<AsyncFactory>>().unwrap();
    assert!(Arc::strong_count(&service) >= 1);
}

#[tokio::test]
async fn test_async_factory_with_result() {
    let app = App::builder()
        .add_service::<AsyncFactoryWithResult>()
        .build()
        .await
        .unwrap();

    let service = app.get_component::<Arc<AsyncFactoryWithResult>>().unwrap();
    assert!(Arc::strong_count(&service) >= 1);
}

#[tokio::test]
async fn test_custom_handle_types() {
    let app = App::builder()
        .add_service::<SimpleFactory>()
        .add_service::<FactoryWithCustomHandle>()
        .build()
        .await
        .unwrap();

    let arc_factory = app.get_component::<Arc<SimpleFactory>>().unwrap();
    assert!(Arc::strong_count(&arc_factory) >= 1);

    let int_handle = app.get_component::<i32>().unwrap();
    assert_eq!(int_handle, 42);
}

#[tokio::test]
async fn test_factory_with_result_and_deps() {
    let app = App::builder()
        .add_service::<SimpleService>()
        .add_service::<FactoryWithResultAndDeps>()
        .build()
        .await
        .unwrap();

    let service = app
        .get_component::<Arc<FactoryWithResultAndDeps>>()
        .unwrap();
    assert!(Arc::strong_count(&service) >= 1);
}

// Example combining derive(Service) for simple structs and #[service] for factory pattern
#[derive(Service)]
struct DatabaseConnection {
    config_service: Arc<ConfigService>,
}

struct ConfigService {
    connection_string: String,
}

#[service]
impl ConfigService {
    #[factory]
    async fn new() -> Result<Arc<Self>, std::io::Error> {
        Ok(Arc::new(Self {
            connection_string: "postgresql://localhost/test".to_string(),
        }))
    }
}

#[tokio::test]
async fn test_combined_derive_and_factory() {
    let app = App::builder()
        .add_service::<ConfigService>()
        .add_service::<DatabaseConnection>()
        .build()
        .await
        .unwrap();

    let config = app.get_component::<Arc<ConfigService>>().unwrap();
    assert_eq!(config.connection_string, "postgresql://localhost/test");

    let db = app.get_component::<Arc<DatabaseConnection>>().unwrap();
    assert!(Arc::strong_count(&db.config_service) >= 2);
}

// Test custom error types that implement Into<StdError>
#[derive(Debug)]
struct CustomError(String);

impl std::fmt::Display for CustomError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Custom error: {}", self.0)
    }
}

impl std::error::Error for CustomError {}

struct ServiceWithCustomError;

#[service]
impl ServiceWithCustomError {
    #[factory]
    async fn new() -> Result<Arc<Self>, CustomError> {
        Ok(Arc::new(Self))
    }
}

#[tokio::test]
async fn test_factory_with_custom_error() {
    let app = App::builder()
        .add_service::<ServiceWithCustomError>()
        .build()
        .await
        .unwrap();

    let service = app.get_component::<Arc<ServiceWithCustomError>>().unwrap();
    assert!(Arc::strong_count(&service) >= 1);
}
