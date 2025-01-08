use clap::Parser;
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
use strum::IntoEnumIterator;
use strum_macros::EnumIter;
use thiserror::Error;
use tokio_stream::StreamExt;
use xcommand::StdioType;
use xcommand::XCommand;
use xcommand::XStatus;
mod config;
use config::Config;

// TODO: make indent configurable
const INDENT: &str = "   ";

#[derive(Debug, Eq, PartialEq, EnumIter)]
enum Command {
    Format,
    Check,
    Build,
    Test,
    Run,
}

#[derive(Debug, Error)]
#[error("unexpected command name")]
struct CommandError {}

impl FromStr for Command {
    type Err = CommandError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let command = match s {
            "f" | "format" => Self::Format,
            "c" | "check" => Self::Check,
            "b" | "build" => Self::Build,
            "t" | "test" => Self::Test,
            "r" | "run" => Self::Run,
            _ => return Err(CommandError {}),
        };
        Ok(command)
    }
}

#[derive(Debug, Parser)]
struct Cli {
    /// Print what command would be ran without actually running it
    #[clap(short, long)]
    dry_run: bool,
}

impl Display for Command {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let s = match self {
            Command::Format => "f/format",
            Command::Check => "c/check",
            Command::Build => "b/build",
            Command::Test => "t/test",
            Command::Run => "r/run",
        };
        write!(f, "{}", s)
    }
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

    let config_file = config_dir.join("config.toml");
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

    // Check multicall program name
    // TODO: checkout clap's multicall support
    let valid_options: Vec<_> = Command::iter().map(|c| c.to_string()).collect();
    let num_options = valid_options.len();
    let last_index = num_options - 1;
    let mut options = String::from("[");
    for (i, op) in valid_options.iter().enumerate() {
        options.push_str(op);
        if i < last_index {
            options.push_str(", ");
        }
    }
    options.push(']');

    let program = env::args().next().unwrap();
    let program_path = PathBuf::from(program);
    let program_name = program_path.file_name().unwrap().to_str().unwrap();
    if program_name == "cbtr" {
        error!(
            "The cbtr program is expected to be invoked as one of {}",
            options
        );
        std::process::exit(2);
    };
    let Ok(command) = Command::from_str(program_name) else {
        error!(
            "cbtr multicall program invoked with unexpected name '{}'. Valid options are {}",
            program_name, options
        );
        std::process::exit(2);
    };

    let cwd = env::current_dir()?;
    let root = match repo_root(&cwd) {
        Ok(root) => root,
        Err(_) => {
            // Fall back to cwd if we aren't working in a git repo
            warn!("Current dir is not within a git repo. Using CWD as repo root");
            cwd.clone()
        }
    };

    let repo_config_file = root.join(".cbtr.toml");
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
                Command::Format => {
                    tools = entry.tools.format.as_ref();
                }
                Command::Check => {
                    tools = entry.tools.check.as_ref();
                }
                Command::Build => {
                    tools = entry.tools.build.as_ref();
                }
                Command::Test => {
                    tools = entry.tools.test.as_ref();
                }
                Command::Run => {
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
