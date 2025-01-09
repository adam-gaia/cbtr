use clap::{Args, Parser, Subcommand};
use color_eyre::eyre::bail;
use color_eyre::Result;
use directories::ProjectDirs;
use env_logger::Env;
use log::debug;
use log::info;
use log::Level;
use log::{error, warn};
use owo_colors::OwoColorize;
use std::env;
use std::fmt;
use std::fmt::Display;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use thiserror::Error;
use tokio_stream::StreamExt;
use xcommand::StdioType;
use xcommand::XCommand;
use xcommand::XStatus;
mod config;
use config::Config;

// TODO: make indent configurable
const INDENT: &str = "   ";
const USER_CONFIG_NAME: &str = "config.toml";
const REPO_CONFIG_NAME: &str = ".cbtr.toml"; // TODO: make configurable

#[derive(Debug, Args, Clone)]
struct CommandArgs {
    /// Print what command would be ran without actually running it
    #[arg(short, long)]
    dry_run: bool,

    /// Only search CWD for file rules (do not search between CWD and repo root)
    #[arg(short, long)]
    no_searchback: bool,
}

#[derive(Subcommand, Debug, Clone)]
enum Command {
    #[command(id = "f")]
    Format {
        #[clap(flatten)]
        args: CommandArgs,
    },
    #[command(id = "c")]
    Check {
        #[clap(flatten)]
        args: CommandArgs,
    },
    #[command(id = "b")]
    Build {
        #[clap(flatten)]
        args: CommandArgs,
    },
    #[command(id = "t")]
    Test {
        #[clap(flatten)]
        args: CommandArgs,
    },
    #[command(id = "r")]
    Run {
        #[clap(flatten)]
        args: CommandArgs,
    },
}

impl Command {
    fn args(&self) -> &CommandArgs {
        match self {
            Command::Format { args } => args,
            Command::Check { args } => args,
            Command::Build { args } => args,
            Command::Test { args } => args,
            Command::Run { args } => args,
        }
    }
}

impl Display for Command {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Command::Format { args: _ } => "format",
            Command::Check { args: _ } => "check",
            Command::Build { args: _ } => "build",
            Command::Test { args: _ } => "test",
            Command::Run { args: _ } => "run",
        };
        write!(f, "{}", s)
    }
}

#[derive(Subcommand, Debug)]
enum Multicall {
    #[command(flatten)]
    Multicall(Command),
    Cbtr {
        #[command(subcommand)]
        command: Command,
    },
}

#[derive(Debug, Error)]
#[error("unexpected command name")]
struct CommandError {}

#[derive(Debug, Parser)]
#[command(multicall(true))]
struct Cli {
    #[command(subcommand)]
    multicall: Multicall,
}

fn repo_root(cwd: &Path) -> Result<PathBuf> {
    let repo = gix::discover(cwd)?;
    let git_dir = repo.path();
    let root = git_dir.parent().unwrap();
    Ok(root.to_path_buf())
}

async fn run(cmd: &str, args: &[&str]) -> Result<i32> {
    let bin = which::which(cmd)?;
    let command = XCommand::builder(&bin)?.args(args)?.build();
    let Ok(mut child) = command.spawn() else {
        bail!("Unable to run '{}'", bin.display());
    };

    // Loop over stdout/err output from the child process
    let mut streamer = child.streamer();
    let mut stream = streamer.stream();
    while let Some(item) = stream.next().await {
        let (message_type, message) = item?;
        match message_type {
            StdioType::Stdout => {
                println!("{}{}", INDENT, message);
            }
            StdioType::Stderr => {
                eprintln!("{}{}", INDENT, message);
            }
        }
    }

    // Grab the exit code of the process
    let XStatus::Exited(code) = child.status().await? else {
        bail!("Process was expected to have finished");
    };
    Ok(code)
}

fn user_config() -> Result<Config> {
    let Some(proj_dirs) = ProjectDirs::from("", "", "cbtr") else {
        bail!("Couldn't find proj dirs");
    };

    // TODO: config dir should contain multiple tomls, where each toml could share the same 'entry.file' or 'entry.bin'
    let config_dir = proj_dirs.config_dir();
    if !config_dir.is_dir() {
        fs::create_dir_all(config_dir)?;
    }

    let config_file = config_dir.join(USER_CONFIG_NAME);
    if !config_file.is_file() {
        bail!("Please create a cbtr config at {}", config_file.display());
    };

    let contents = fs::read_to_string(&config_file)?;
    let config: Config = toml::from_str(&contents)?;
    Ok(config)
}

#[tokio::main]
async fn main() -> Result<()> {
    let log_level = Env::default().default_filter_or("info");
    env_logger::Builder::from_env(log_level)
        .format(|buf, record| {
            let level_value = record.level();
            let level = format!("[{}]", level_value);

            let level = match level_value {
                Level::Error => format!("{}", level.red()),
                Level::Warn => format!("{}", level.yellow()),
                Level::Info => format!("{}", level.green()),
                Level::Debug => format!("{}", level.blue()),
                Level::Trace => format!("{}", level.cyan()),
            };

            writeln!(buf, "{} {}", level.bold(), record.args())
        })
        .init();

    let args = Cli::parse();
    let command = match &args.multicall {
        Multicall::Multicall(c) => c,
        Multicall::Cbtr { command } => command,
    };
    debug!("args: {:?}", args);
    let args = command.args();

    let cwd = env::current_dir()?;
    let root = if args.no_searchback {
        // Stop searchback by making repo_root == cwd
        cwd.clone()
    } else {
        match repo_root(&cwd) {
            Ok(root) => root,
            Err(_) => {
                // Fall back to cwd if we aren't working in a git repo
                warn!("Current dir is not within a git repo. Using CWD as repo root");
                cwd.clone()
            }
        }
    };

    let repo_config_file = root.join(REPO_CONFIG_NAME);
    let repo_config = if repo_config_file.is_file() {
        let contents = fs::read_to_string(&repo_config_file)?;
        let config: Config = toml::from_str(&contents)?;
        Some(config)
    } else {
        None
    };

    let user_config = match user_config() {
        Ok(config) => Some(config),
        Err(_) => None,
    };

    let config = match (repo_config, user_config) {
        (Some(mut repo_config), Some(user_config)) => {
            repo_config.append(user_config);
            repo_config
        }
        (Some(config), None) => config,
        (None, Some(config)) => config,
        (None, None) => {
            //
            error!("Could not find config file");
            std::process::exit(1);
        }
    };

    let mut tools = None;
    for entry in &config.entries {
        let name = &entry.name;
        debug!("Checking conditions for {}", name);

        if entry.matches(&cwd, &root) {
            match command {
                Command::Format { args: _ } => {
                    tools = entry.tools.format.as_ref();
                }
                Command::Check { args: _ } => {
                    tools = entry.tools.check.as_ref();
                }
                Command::Build { args: _ } => {
                    tools = entry.tools.build.as_ref();
                }
                Command::Test { args: _ } => {
                    tools = entry.tools.test.as_ref();
                }
                Command::Run { args: _ } => {
                    tools = entry.tools.run.as_ref();
                }
            }

            if tools.is_some() {
                break;
            }
        }
    }

    let Some(tools) = tools else {
        error!("No {} tool matched config rules", command);
        std::process::exit(1);
    };

    for tool in tools.to_vec() {
        info!("Running '{}'", tool.bold());

        if args.dry_run {
            println!("[dryrun] Would run '{}'", tool)
        } else {
            let parts: Vec<&str> = tool.split_whitespace().collect();
            let cmd = parts[0];
            let cmd_args = &parts[1..];
            let code = run(cmd, cmd_args).await?;
            debug!("cmd: {}, args: {:?}", cmd, cmd_args);
            if code != 0 {
                error!("Subprocess '{}' failed with exit code {}", tool, code);
                std::process::exit(code)
            };
        }
    }

    Ok(())
}
