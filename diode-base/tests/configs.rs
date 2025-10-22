use diode::Extract;
use diode_base::{Config, ConfigSection, config_section};
use serde::{Deserialize, Serialize};
use std::fs;
use tempfile::NamedTempFile;
use tokio;

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct TestConfig {
    name: String,
    port: u16,
    enabled: bool,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct DatabaseConfig {
    host: String,
    port: u16,
    ssl: bool,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct ServerConfig {
    bind_addr: String,
    workers: u32,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct CacheConfig {
    enabled: bool,
}

#[config_section("test_section")]
#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct TestSectionConfig {
    name: String,
    value: i32,
}

#[config_section("database")]
#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct DatabaseSectionConfig {
    host: String,
    port: u16,
    ssl: bool,
}

#[tokio::test]
async fn test_config_new() {
    let config = Config::new();
    assert!(config.is_empty());
}

#[tokio::test]
async fn test_config_default() {
    let config = Config::default();
    assert!(config.is_empty());
}

#[tokio::test]
async fn test_config_set_and_get() {
    let mut config = Config::new();

    let test_config = TestConfig {
        name: "test_app".to_string(),
        port: 8080,
        enabled: true,
    };

    // Test setting a value
    config.set("app", &test_config).unwrap();

    // Test getting the value back
    let retrieved: TestConfig = config.get("app").unwrap();
    assert_eq!(retrieved, test_config);
}

#[tokio::test]
async fn test_config_get_nonexistent() {
    let config = Config::new();

    // Getting non-existent key should return null/default value
    let result: Option<String> = config.get("nonexistent").unwrap();
    assert_eq!(result, None);
}

#[tokio::test]
async fn test_config_with() {
    let test_config = TestConfig {
        name: "test_app".to_string(),
        port: 8080,
        enabled: true,
    };

    let config = Config::new().with("app", &test_config);

    let retrieved: TestConfig = config.get("app").unwrap();
    assert_eq!(retrieved, test_config);
}

#[tokio::test]
async fn test_config_parse_from_string() {
    let json_str = r#"
    {
        "app": {
            "name": "test_app",
            "port": 8080,
            "enabled": true
        }
    }
    "#;

    let config = Config::parse(json_str).unwrap();
    let app_config: TestConfig = config.get("app").unwrap();

    assert_eq!(app_config.name, "test_app");
    assert_eq!(app_config.port, 8080);
    assert_eq!(app_config.enabled, true);
}

#[tokio::test]
async fn test_config_parse_invalid_json() {
    let invalid_json = r#"{ "invalid": json }"#;

    let result = Config::parse(invalid_json);
    assert!(result.is_err());
}

#[tokio::test]
async fn test_config_parse_file() {
    let json_content = r#"
    {
        "server": {
            "bind_addr": "127.0.0.1:8080",
            "workers": 4
        },
        "database": {
            "host": "localhost",
            "port": 5432,
            "ssl": true
        }
    }
    "#;

    // Create a temporary file
    let temp_file = NamedTempFile::new().unwrap();
    fs::write(temp_file.path(), json_content).unwrap();

    let config = Config::parse_file(temp_file.path()).await.unwrap();
    let server_config: ServerConfig = config.get("server").unwrap();
    let database_config: DatabaseConfig = config.get("database").unwrap();

    assert_eq!(server_config.bind_addr, "127.0.0.1:8080");
    assert_eq!(server_config.workers, 4);
    assert_eq!(database_config.host, "localhost");
    assert_eq!(database_config.port, 5432);
    assert_eq!(database_config.ssl, true);
}

#[tokio::test]
async fn test_config_parse_file_not_found() {
    let result = Config::parse_file("nonexistent_file.json").await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_config_merge_objects() {
    let mut base_config = Config::parse(
        r#"
    {
        "server": {
            "bind_addr": "localhost:8080",
            "workers": 1
        },
        "database": {
            "host": "localhost"
        }
    }
    "#,
    )
    .unwrap();

    let override_config = Config::parse(
        r#"
    {
        "server": {
            "bind_addr": "0.0.0.0:8080",
            "workers": 4
        },
        "database": {
            "port": 5432,
            "ssl": true
        },
        "cache": {
            "enabled": true
        }
    }
    "#,
    )
    .unwrap();

    base_config.merge_from(override_config).unwrap();

    // Check merged values by getting the typed objects
    let server_config: ServerConfig = base_config.get("server").unwrap();
    let database_config: DatabaseConfig = base_config.get("database").unwrap();
    let cache_config: CacheConfig = base_config.get("cache").unwrap();

    assert_eq!(server_config.bind_addr, "0.0.0.0:8080");
    assert_eq!(server_config.workers, 4);

    assert_eq!(database_config.host, "localhost");
    assert_eq!(database_config.port, 5432);
    assert_eq!(database_config.ssl, true);

    assert_eq!(cache_config.enabled, true);
}

#[tokio::test]
async fn test_config_merge_arrays() {
    let mut base_config = Config::parse(
        r#"
    {
        "tags": ["production", "web"]
    }
    "#,
    )
    .unwrap();

    let override_config = Config::parse(
        r#"
    {
        "tags": ["monitoring", "logging"]
    }
    "#,
    )
    .unwrap();

    base_config.merge_from(override_config).unwrap();

    let tags: Vec<String> = base_config.get("tags").unwrap();
    assert_eq!(tags, vec!["production", "web", "monitoring", "logging"]);
}

#[tokio::test]
async fn test_config_merge_replace_primitives() {
    let mut base_config = Config::parse(
        r#"
    {
        "port": 8080,
        "enabled": false,
        "name": "old_name"
    }
    "#,
    )
    .unwrap();

    let override_config = Config::parse(
        r#"
    {
        "port": 9090,
        "enabled": true,
        "name": "new_name"
    }
    "#,
    )
    .unwrap();

    base_config.merge_from(override_config).unwrap();

    let port: u16 = base_config.get("port").unwrap();
    let enabled: bool = base_config.get("enabled").unwrap();
    let name: String = base_config.get("name").unwrap();

    assert_eq!(port, 9090);
    assert_eq!(enabled, true);
    assert_eq!(name, "new_name");
}

#[tokio::test]
async fn test_config_set_different_types() {
    let mut config = Config::new();

    // Test setting different types
    config.set("string_value", "hello").unwrap();
    config.set("number_value", 42i32).unwrap();
    config.set("bool_value", true).unwrap();
    config.set("array_value", vec![1, 2, 3]).unwrap();

    // Test getting them back
    let string_val: String = config.get("string_value").unwrap();
    let number_val: i32 = config.get("number_value").unwrap();
    let bool_val: bool = config.get("bool_value").unwrap();
    let array_val: Vec<i32> = config.get("array_value").unwrap();

    assert_eq!(string_val, "hello");
    assert_eq!(number_val, 42);
    assert_eq!(bool_val, true);
    assert_eq!(array_val, vec![1, 2, 3]);
}

#[tokio::test]
async fn test_config_type_conversion_error() {
    let mut config = Config::new();
    config.set("string_value", "not_a_number").unwrap();

    // Trying to get string as number should fail
    let result: Result<i32, _> = config.get("string_value");
    assert!(result.is_err());
}

#[tokio::test]
async fn test_config_serialization() {
    let mut config = Config::new();
    config.set("app_name", "test_app").unwrap();
    config.set("version", "1.0.0").unwrap();
    config.set("port", 8080u16).unwrap();

    // Test serialization
    let serialized = serde_json::to_string(&config).unwrap();

    // Test deserialization
    let deserialized: Config = serde_json::from_str(&serialized).unwrap();

    let app_name: String = deserialized.get("app_name").unwrap();
    let version: String = deserialized.get("version").unwrap();
    let port: u16 = deserialized.get("port").unwrap();

    assert_eq!(app_name, "test_app");
    assert_eq!(version, "1.0.0");
    assert_eq!(port, 8080);
}

#[tokio::test]
async fn test_config_empty_merge() {
    let mut config = Config::parse(r#"{"key": "value"}"#).unwrap();
    let empty_config = Config::new();

    config.merge_from(empty_config).unwrap();

    let value: String = config.get("key").unwrap();
    assert_eq!(value, "value");
}

#[tokio::test]
async fn test_config_complex_nested_structure() {
    let complex_json = r#"
    {
        "application": {
            "name": "my-app",
            "version": "1.0.0",
            "features": {
                "auth": {
                    "enabled": true,
                    "providers": ["oauth", "saml"]
                },
                "logging": {
                    "level": "info",
                    "outputs": ["console", "file"]
                }
            }
        },
        "infrastructure": {
            "database": {
                "primary": {
                    "host": "db1.example.com",
                    "port": 5432
                },
                "replica": {
                    "host": "db2.example.com",
                    "port": 5432
                }
            }
        }
    }
    "#;

    let config = Config::parse(complex_json).unwrap();

    // Test nested access by getting objects and then accessing their fields
    let application: serde_json::Value = config.get("application").unwrap();
    let infrastructure: serde_json::Value = config.get("infrastructure").unwrap();

    assert_eq!(application["name"].as_str().unwrap(), "my-app");
    assert_eq!(
        application["features"]["auth"]["enabled"]
            .as_bool()
            .unwrap(),
        true
    );
    let providers: Vec<&str> = application["features"]["auth"]["providers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(providers, vec!["oauth", "saml"]);
    assert_eq!(
        infrastructure["database"]["primary"]["host"]
            .as_str()
            .unwrap(),
        "db1.example.com"
    );
}

#[tokio::test]
async fn test_config_section_macro() {
    // Test that the macro correctly generates the key() method
    assert_eq!(TestSectionConfig::key(), "test_section");
    assert_eq!(DatabaseSectionConfig::key(), "database");

    // Test that the macro works with actual config
    let config = Config::parse(
        r#"
    {
        "test_section": {
            "name": "test_name",
            "value": 42
        },
        "database": {
            "host": "localhost",
            "port": 5432,
            "ssl": true
        }
    }
    "#,
    )
    .unwrap();

    // Test getting config sections using the generated key
    let test_section: TestSectionConfig = config.get(TestSectionConfig::key()).unwrap();
    let database_section: DatabaseSectionConfig = config.get(DatabaseSectionConfig::key()).unwrap();

    assert_eq!(test_section.name, "test_name");
    assert_eq!(test_section.value, 42);

    assert_eq!(database_section.host, "localhost");
    assert_eq!(database_section.port, 5432);
    assert_eq!(database_section.ssl, true);
}

#[tokio::test]
async fn test_config_section_macro_with_injection() {
    use diode::App;

    // Create config with test data
    let config = Config::parse(
        r#"
    {
        "test_section": {
            "name": "injected_name",
            "value": 123
        },
        "database": {
            "host": "injected_host",
            "port": 3306,
            "ssl": false
        }
    }
    "#,
    )
    .unwrap();

    // Create app with config
    let mut app_builder = App::builder();
    app_builder.add_component(config);

    // Test extraction using dependency injection
    let test_section: TestSectionConfig = diode_base::Config::extract(&app_builder).unwrap();
    let database_section: DatabaseSectionConfig =
        diode_base::Config::extract(&app_builder).unwrap();

    // Verify extracted data
    assert_eq!(test_section.name, "injected_name");
    assert_eq!(test_section.value, 123);

    assert_eq!(database_section.host, "injected_host");
    assert_eq!(database_section.port, 3306);
    assert_eq!(database_section.ssl, false);
}
