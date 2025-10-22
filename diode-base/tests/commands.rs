use clap::{Arg, ArgMatches, Command as ClapCommand};
use diode::App;
use diode_base::{AddCommandExt, Command, CommandRegistry, Config, ConfigCommand, ServerCommand};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;

// Mock command for testing
struct MockCommand;

impl Command for MockCommand {
    fn command() -> ClapCommand {
        ClapCommand::new("mock")
            .about("Mock command for testing")
            .arg(Arg::new("test-arg").long("test-arg").help("Test argument"))
    }

    async fn main(_app: Arc<App>, matches: ArgMatches) -> ExitCode {
        // Check if test-arg was provided
        if matches.try_contains_id("test-arg").unwrap_or(false) {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        }
    }
}

// Another mock command with different behavior
struct AnotherMockCommand;

impl Command for AnotherMockCommand {
    fn command() -> ClapCommand {
        ClapCommand::new("another")
            .about("Another mock command")
            .arg(Arg::new("required").long("required").required(true))
    }

    async fn main(_app: Arc<App>, _matches: ArgMatches) -> ExitCode {
        ExitCode::SUCCESS
    }
}

struct DefaultCommand;

impl Command for DefaultCommand {
    fn command() -> ClapCommand {
        ClapCommand::new("default")
    }
}

// Mock command that takes time to execute
struct SlowCommand;

impl Command for SlowCommand {
    fn command() -> ClapCommand {
        ClapCommand::new("slow").about("Slow command for testing")
    }

    async fn main(_app: Arc<App>, _matches: ArgMatches) -> ExitCode {
        sleep(Duration::from_millis(100)).await;
        ExitCode::SUCCESS
    }
}

#[tokio::test]
async fn test_command_registry_new() {
    let registry = CommandRegistry::default();
    assert_eq!(registry.len(), 0);
}

#[tokio::test]
async fn test_command_registry_add_command() {
    let mut registry = CommandRegistry::default();
    registry.add_command::<MockCommand>();
    assert_eq!(registry.len(), 1);
}

#[tokio::test]
async fn test_command_registry_add_multiple_commands() {
    let mut registry = CommandRegistry::default();
    registry.add_command::<MockCommand>();
    registry.add_command::<AnotherMockCommand>();
    assert_eq!(registry.len(), 2);
}

#[tokio::test]
async fn test_command_registry_build_cli() {
    let mut registry = CommandRegistry::default();
    registry.add_command::<MockCommand>();
    registry.add_command::<AnotherMockCommand>();

    let cli = registry.build_cli();

    // Check that subcommands are present
    let subcommands: Vec<_> = cli.get_subcommands().map(|s| s.get_name()).collect();
    assert!(subcommands.contains(&"mock"));
    assert!(subcommands.contains(&"another"));

    // Check that global arguments are present
    assert!(cli.get_arguments().any(|arg| arg.get_id() == "config"));
    assert!(
        cli.get_arguments()
            .any(|arg| arg.get_id() == "config-override")
    );
}

#[tokio::test]
async fn test_command_registry_build_cli_requires_subcommand() {
    let registry = CommandRegistry::default();
    let cli = registry.build_cli();
    assert!(cli.is_subcommand_required_set());
}

#[tokio::test]
async fn test_server_command_definition() {
    let cmd = ServerCommand::command();
    assert_eq!(cmd.get_name(), "server");
}

#[tokio::test]
async fn test_config_command_definition() {
    let cmd = ConfigCommand::command();
    assert_eq!(cmd.get_name(), "config");
}

#[tokio::test]
async fn test_config_command_execution() {
    // Create a test config
    let mut config = Config::new();
    config.set("app_name", "test_app").unwrap();
    config.set("version", "1.0.0").unwrap();

    // Create app with config
    let mut app_builder = App::builder();
    app_builder.add_component(config);
    let app = Arc::new(app_builder.build().await.unwrap());

    // Execute config command
    let matches = ArgMatches::default();
    let exit_code = ConfigCommand::main(app, matches).await;

    assert_eq!(exit_code, ExitCode::SUCCESS);
}

#[tokio::test]
async fn test_add_command_ext() {
    let mut app_builder = App::builder();
    assert!(!app_builder.has_command::<MockCommand>());
    app_builder.add_command::<MockCommand>();
    assert!(app_builder.has_command::<MockCommand>());
}

#[tokio::test]
async fn test_add_command_ext_multiple() {
    let mut app_builder = App::builder();
    app_builder
        .add_command::<MockCommand>()
        .add_command::<AnotherMockCommand>();
    let registry = app_builder.get_component_ref::<CommandRegistry>().unwrap();
    assert_eq!(registry.len(), 2);
}

#[tokio::test]
async fn test_add_command_ext_creates_registry() {
    let mut app_builder = App::builder();
    assert!(!app_builder.has_component::<CommandRegistry>());
    app_builder.add_command::<MockCommand>();
    assert!(app_builder.has_component::<CommandRegistry>());
}

#[tokio::test]
async fn test_default_command_implementation() {
    let app = Arc::new(App::builder().build().await.unwrap());
    let matches = ArgMatches::default();
    let exit_code = DefaultCommand::main(app, matches).await;

    // Default implementation returns FAILURE
    assert_eq!(exit_code, ExitCode::FAILURE);
}

#[tokio::test]
async fn test_command_registry_with_real_app() {
    let mut app_builder = App::builder();
    app_builder.add_command::<MockCommand>();

    let registry = app_builder.get_component_ref::<CommandRegistry>().unwrap();
    let cli = registry.build_cli();

    // Verify CLI structure
    assert!(cli.get_subcommands().any(|s| s.get_name() == "mock"));
    assert!(cli.get_arguments().any(|arg| arg.get_id() == "config"));
}

#[tokio::test]
async fn test_slow_command_execution() {
    let app = Arc::new(App::builder().build().await.unwrap());
    let matches = ArgMatches::default();

    let start = std::time::Instant::now();
    let exit_code = SlowCommand::main(app, matches).await;
    let duration = start.elapsed();

    assert_eq!(exit_code, ExitCode::SUCCESS);
    assert!(duration >= Duration::from_millis(50)); // Should take some time
}
