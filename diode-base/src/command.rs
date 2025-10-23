//! Command-line interface framework for diode applications.
//!
//! This module provides a framework for building CLI applications with multiple subcommands
//! that can access the dependency injection container. Commands are registered with the
//! application and can be executed through a unified CLI interface.
//!
//! # Core Concepts
//!
//! - **Command**: A trait for defining CLI subcommands
//! - **CommandRegistry**: Container for all registered commands
//! - **CLI Integration**: Automatic integration with clap for argument parsing
//!
//! # Examples
//!
//! Basic command implementation:
//!
//! ```rust
//! use diode_base::{Command, AddCommandExt};
//! use diode::App;
//! use clap::{ArgMatches, Command as ClapCommand};
//! use std::process::ExitCode;
//! use std::sync::Arc;
//!
//! struct HelloCommand;
//!
//! impl Command for HelloCommand {
//!     fn command() -> ClapCommand {
//!         ClapCommand::new("hello")
//!             .about("Prints a greeting")
//!     }
//!
//!     async fn main(_app: Arc<App>, _matches: ArgMatches) -> ExitCode {
//!         println!("Hello, World!");
//!         ExitCode::SUCCESS
//!     }
//! }
//! ```

use std::any::TypeId;
use std::collections::{BTreeMap, HashMap};
use std::marker::PhantomData;
use std::mem::take;
use std::process::ExitCode;
use std::sync::Arc;

use async_trait::async_trait;
use clap::{Arg, ArgAction, ArgMatches};
use diode::{App, AppBuilder};

use crate::{CancellationToken, Config, Metrics, RunDaemonsExt, Tracing};

/// Trait for defining CLI commands that can access the application's dependency container.
///
/// Commands are subcommands in the CLI that can perform operations using services
/// and components from the application. Each command defines its CLI interface
/// and main execution logic.
///
/// # Examples
///
/// Simple command:
///
/// ```rust
/// use diode_base::Command;
/// use diode::App;
/// use clap::{ArgMatches, Command as ClapCommand};
/// use std::process::ExitCode;
/// use std::sync::Arc;
///
/// struct StatusCommand;
///
/// impl Command for StatusCommand {
///     fn command() -> ClapCommand {
///         ClapCommand::new("status")
///             .about("Shows application status")
///     }
///
///     async fn main(app: Arc<App>, _matches: ArgMatches) -> ExitCode {
///         // Access services from the app container
///         println!("Application is running");
///         ExitCode::SUCCESS
///     }
/// }
/// ```
///
/// Command with arguments:
///
/// ```rust
/// use diode_base::Command;
/// use diode::App;
/// use clap::{Arg, ArgMatches, Command as ClapCommand};
/// use std::process::ExitCode;
/// use std::sync::Arc;
///
/// struct GreetCommand;
///
/// impl Command for GreetCommand {
///     fn command() -> ClapCommand {
///         ClapCommand::new("greet")
///             .about("Greets a user")
///             .arg(Arg::new("name")
///                 .help("Name to greet")
///                 .required(true))
///     }
///
///     async fn main(_app: Arc<App>, matches: ArgMatches) -> ExitCode {
///         let name = matches.get_one::<String>("name").unwrap();
///         println!("Hello, {}!", name);
///         ExitCode::SUCCESS
///     }
/// }
/// ```
pub trait Command: Send + Sync {
    /// Defines the CLI command structure for this command.
    ///
    /// This method should return a `clap::Command` that defines the command name,
    /// description, arguments, and other CLI options.
    ///
    /// # Returns
    ///
    /// A `clap::Command` instance describing this command's CLI interface.
    fn command() -> clap::Command
    where
        Self: Sized;

    /// Executes the command with the given application and parsed arguments.
    ///
    /// This is the main entry point for command execution. The method receives
    /// the application container and the parsed command-line arguments.
    ///
    /// # Arguments
    ///
    /// * `app` - Shared reference to the application container
    /// * `matches` - Parsed command-line arguments for this command
    ///
    /// # Returns
    ///
    /// Returns an `ExitCode` indicating the command's execution result.
    fn main(
        app: Arc<App>,
        matches: ArgMatches,
    ) -> impl std::future::Future<Output = ExitCode> + Send {
        let _ = (app, matches);
        async move { ExitCode::FAILURE }
    }
}

#[async_trait]
trait DynCommand: Send + Sync {
    fn command(&self) -> clap::Command;

    async fn main(&self, app: Arc<App>, matches: ArgMatches) -> ExitCode;
}

#[async_trait]
impl<T> DynCommand for T
where
    T: Command,
{
    fn command(&self) -> clap::Command {
        T::command()
    }

    async fn main(&self, app: Arc<App>, matches: ArgMatches) -> ExitCode {
        T::main(app, matches).await
    }
}

/// Registry for managing all commands in the application.
///
/// The `CommandRegistry` stores all registered commands and provides functionality
/// for building the CLI interface and executing commands. It's automatically
/// managed by the application builder when commands are registered.
///
/// # Examples
///
/// ```rust
/// use diode_base::{CommandRegistry, Command, AddCommandExt};
/// use diode::App;
///
/// struct MyCommand;
/// impl Command for MyCommand {
///     fn command() -> clap::Command { clap::Command::new("my-cmd") }
/// }
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let app = App::builder()
///     .add_command::<MyCommand>()
///     .build()
///     .await?;
/// # Ok(())
/// # }
/// ```
#[derive(Default)]
#[doc(hidden)]
pub struct CommandRegistry {
    commands: HashMap<TypeId, Box<dyn DynCommand>>,
}

impl CommandRegistry {
    /// Registers a command type with the registry.
    ///
    /// # Type Parameters
    ///
    /// * `T` - The command type to register. Must implement `Command + 'static`.
    pub fn add_command<T>(&mut self)
    where
        T: Command + 'static,
    {
        let type_id = TypeId::of::<T>();
        self.commands
            .insert(type_id, Box::new(CommandWrapper::<T>(PhantomData)));
    }

    /// Checks if a command type has been registered.
    ///
    /// # Type Parameters
    ///
    /// * `T` - The command type to check for.
    ///
    /// # Returns
    ///
    /// Returns `true` if the command is registered, `false` otherwise.
    pub fn has_command<T>(&self) -> bool
    where
        T: Command + 'static,
    {
        let type_id = TypeId::of::<T>();
        self.commands.contains_key(&type_id)
    }

    /// Builds the complete CLI interface with all registered commands.
    ///
    /// Creates a `clap::Command` that includes all registered commands as subcommands
    /// and sets up common CLI options like config file paths.
    ///
    /// # Returns
    ///
    /// A `clap::Command` configured with all registered subcommands.
    pub fn build_cli(&self) -> clap::Command {
        let mut cli = clap::Command::default()
            .subcommand_required(true)
            .arg(Arg::new("config").long("config").short('c').required(true))
            .arg(
                Arg::new("config-override")
                    .long("config-override")
                    .short('o')
                    .action(ArgAction::Append),
            );
        let mut commands = BTreeMap::new();
        for command in self.commands.values() {
            let subcmd = command.command();
            commands.insert(subcmd.get_name().to_owned(), command);
            cli = cli.subcommand(subcmd);
        }
        cli
    }

    /// Executes the appropriate command based on parsed CLI arguments.
    ///
    /// # Arguments
    ///
    /// * `app` - Shared reference to the application container
    /// * `matches` - Parsed command-line arguments including subcommand selection
    ///
    /// # Returns
    ///
    /// Returns the exit code from the executed command.
    pub async fn run_main(&self, app: Arc<App>, mut matches: ArgMatches) -> ExitCode {
        let (name, matches) = matches.remove_subcommand().unwrap();
        let command = self
            .commands
            .values()
            .find(|v| v.command().get_name() == name)
            .unwrap();
        command.main(app, matches).await
    }

    /// Returns the number of registered commands.
    ///
    /// # Returns
    ///
    /// The count of registered commands in this registry.
    pub fn len(&self) -> usize {
        self.commands.len()
    }

    /// Checks if the registry has no registered commands.
    ///
    /// # Returns
    ///
    /// Returns `true` if no commands are registered, `false` otherwise.
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }
}

struct CommandWrapper<T>(PhantomData<T>)
where
    T: Command;

impl<T> Command for CommandWrapper<T>
where
    T: Command,
{
    fn command() -> clap::Command
    where
        Self: Sized,
    {
        T::command()
    }

    async fn main(app: Arc<App>, matches: ArgMatches) -> ExitCode {
        T::main(app, matches).await
    }
}

/// Extension trait for `AppBuilder` to add command registration methods.
///
/// This trait provides convenient methods for registering commands with the
/// application builder. Commands registered this way will be available in
/// the CLI interface when the application is run.
pub trait AddCommandExt {
    /// Registers a command with the application builder.
    ///
    /// The command will be available as a subcommand in the CLI interface.
    /// If this is the first command being added, a `CommandRegistry` will
    /// be automatically created and added to the application.
    ///
    /// # Type Parameters
    ///
    /// * `T` - The command type to register. Must implement `Command + 'static`.
    ///
    /// # Returns
    ///
    /// Returns `&mut Self` for method chaining.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use diode::{App, AppBuilder};
    /// use diode_base::{Command, AddCommandExt};
    /// use clap::Command as ClapCommand;
    /// use std::process::ExitCode;
    /// use std::sync::Arc;
    ///
    /// struct MyCommand;
    /// impl Command for MyCommand {
    ///     fn command() -> ClapCommand { ClapCommand::new("my-cmd") }
    /// }
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let app = App::builder()
    ///     .add_command::<MyCommand>()
    ///     .build()
    ///     .await?;
    /// # Ok(())
    /// # }
    /// ```
    fn add_command<T>(&mut self) -> &mut Self
    where
        T: Command + 'static;

    /// Checks if a command type has been registered.
    ///
    /// # Type Parameters
    ///
    /// * `T` - The command type to check for.
    ///
    /// # Returns
    ///
    /// Returns `true` if the command is registered, `false` otherwise.
    fn has_command<T>(&self) -> bool
    where
        T: Command + 'static;
}

impl AddCommandExt for AppBuilder {
    fn add_command<T>(&mut self) -> &mut Self
    where
        T: Command + 'static,
    {
        if !self.has_component::<CommandRegistry>() {
            self.add_component(CommandRegistry::default());
        }
        self.get_component_mut::<CommandRegistry>()
            .unwrap()
            .add_command::<T>();
        self
    }

    fn has_command<T>(&self) -> bool
    where
        T: Command + 'static,
    {
        self.get_component_ref::<CommandRegistry>()
            .is_some_and(|v| v.has_command::<T>())
    }
}

/// Extension trait for `AppBuilder` to run the main CLI application.
///
/// This trait provides the main entry point for CLI applications, handling
/// argument parsing, configuration loading, and command execution.
pub trait RunMainExt {
    /// Runs the main CLI application.
    ///
    /// This method:
    /// 1. Registers default commands (server, config) if not already present
    /// 2. Builds the CLI interface from registered commands
    /// 3. Parses command-line arguments
    /// 4. Loads and merges configuration files
    /// 5. Sets up tracing/logging
    /// 6. Builds the application
    /// 7. Executes the selected command
    ///
    /// # Returns
    ///
    /// Returns the exit code from the executed command.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use diode::App;
    /// use diode_base::RunMainExt;
    ///
    /// #[tokio::main]
    /// async fn main() -> std::process::ExitCode {
    ///     App::builder().run_main().await
    /// }
    /// ```
    fn run_main(&mut self) -> impl std::future::Future<Output = ExitCode> + Send;
}

impl RunMainExt for AppBuilder {
    async fn run_main(&mut self) -> ExitCode {
        if !self.has_command::<ServerCommand>() {
            self.add_command::<ServerCommand>();
        }
        if !self.has_command::<ConfigCommand>() {
            self.add_command::<ConfigCommand>();
        }
        // Setup cli.
        let command_registry = take(self.get_component_mut::<CommandRegistry>().unwrap());
        let cli = command_registry.build_cli();
        let matches = cli.get_matches();
        // Setup config.
        if !self.has_component::<Config>() {
            let config_path = matches.get_one::<String>("config").unwrap();
            let mut config = Config::parse_file(config_path).await.unwrap();
            let config_override_paths = matches
                .get_many::<String>("config-override")
                .unwrap_or_default();
            for path in config_override_paths {
                let config_override = Config::parse_file(path).await.unwrap();
                config.merge_from(config_override).unwrap();
            }
            self.add_component(config);
        }
        // Setup tracing.
        Tracing::build(self).unwrap();
        // Setup metrics.
        Metrics::build(self).unwrap();
        // Start app.
        let app = Arc::new(self.build().await.unwrap());
        command_registry.run_main(app, matches).await
    }
}

/// Built-in server command that runs all registered daemons.
///
/// This command starts the application in server mode, running all registered
/// daemon services until a shutdown signal (Ctrl+C) is received.
pub struct ServerCommand;

impl Command for ServerCommand {
    fn command() -> clap::Command
    where
        Self: Sized,
    {
        clap::Command::new("server")
    }

    async fn main(app: Arc<App>, _matches: ArgMatches) -> ExitCode {
        let shutdown = CancellationToken::new();
        tokio::spawn({
            let shutdown = shutdown.clone();
            async move {
                tokio::signal::ctrl_c()
                    .await
                    .expect("Failed to listen for ctrl_c");
                shutdown.cancel();
            }
        });
        if let Err(err) = app.run_daemons(shutdown).await {
            panic!("Failed to run server: {err}");
        }
        ExitCode::SUCCESS
    }
}

/// Built-in config command that displays the current configuration.
///
/// This command prints the current application configuration in JSON format,
/// useful for debugging configuration loading and merging.
pub struct ConfigCommand;

impl Command for ConfigCommand {
    fn command() -> clap::Command
    where
        Self: Sized,
    {
        clap::Command::new("config")
    }

    async fn main(app: Arc<App>, _matches: ArgMatches) -> ExitCode {
        let config = app.get_component_ref::<Config>().unwrap();
        println!("{}", serde_json::to_string_pretty(&config.configs).unwrap());
        ExitCode::SUCCESS
    }
}
