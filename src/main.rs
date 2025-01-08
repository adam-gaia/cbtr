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
use pathdiff::diff_paths;
use serde::Deserialize;
use std::env;
use std::fmt;
use std::fmt::Display;
use std::fs;
use std::io::Write;
use std::path::Component;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use strum::IntoEnumIterator;
use strum_macros::EnumIter;
use thiserror::Error;
use tokio_stream::StreamExt;
use xcommand::StdioType;
use xcommand::XCommand;
use xcommand::XStatus;

// TODO: make indent configurable
const INDENT: &'static str = "   ";

// TODO: forget the shell script and write the whole thing in rust

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

#[derive(Deserialize, Debug)]
#[serde(untagged)]
enum StringOrVec {
    Single(String),
    Multiple(Vec<String>),
}

impl StringOrVec {
    fn to_vec(&self) -> Vec<String> {
        match self {
            StringOrVec::Single(s) => vec![s.clone()],
            StringOrVec::Multiple(v) => v.clone(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct Tools {
    format: Option<StringOrVec>,
    check: Option<StringOrVec>,
    build: Option<StringOrVec>,
    test: Option<StringOrVec>,
    run: Option<StringOrVec>,
}

#[derive(Debug, Deserialize)]
enum Direction {
    #[serde(rename = "backwards")]
    Backwards,
    #[serde(rename = "forwards")]
    Forwards,
}

impl Default for Direction {
    fn default() -> Self {
        Direction::Forwards
    }
}

#[derive(Debug, Deserialize)]
struct File {
    name: StringOrVec,
    #[serde(rename = "search-direction", default)]
    search_direction: Direction,
}

#[derive(Debug, Deserialize)]
struct Entry {
    name: String,
    bin: Option<StringOrVec>,
    file: Option<File>,
    tools: Tools,
}

/// Search from cwd backwards to repo_root
fn back_search(cwd: &Path, repo_root: &Path, file: &str) -> bool {
    let mut current_dir = cwd.to_path_buf();
    loop {
        let candidate = current_dir.join(file);
        if candidate.is_file() {
            debug!("Found {}", candidate.display());
            return true;
        }

        if current_dir == repo_root {
            break;
        }

        current_dir = current_dir.parent().unwrap().to_path_buf();
    }
    false
}

/// Search from repo_root forward to cwd
fn forward_search(cwd: &Path, repo_root: &Path, file: &str) -> bool {
    if let Some(diff) = diff_paths(cwd, repo_root) {
        let mut path = repo_root.to_path_buf();
        let candidate = path.join(file);
        if candidate.is_file() {
            return true;
        }

        for component in diff.components() {
            match component {
                Component::Normal(s) => {
                    path = path.join(s);
                    let candidate = path.join(file);
                    if candidate.is_file() {
                        return true;
                    }
                }
                Component::CurDir => {
                    continue;
                }
                _ => {
                    return false;
                }
            }
        }
    }

    false
}

fn path_search(cwd: &Path, repo_root: &Path, direction: &Direction, file: &str) -> bool {
    match direction {
        Direction::Backwards => back_search(cwd, repo_root, file),
        Direction::Forwards => forward_search(cwd, repo_root, file),
    }
}

impl Entry {
    fn matches(&self, cwd: &Path, repo_root: &Path) -> bool {
        if let Some(bin) = &self.bin {
            for bin in bin.to_vec() {
                if which::which(&bin).is_err() {
                    debug!("Couldn't find {} on $PATH", bin);
                    return false;
                }
            }
        }

        if let Some(file) = &self.file {
            let direction = &file.search_direction;
            for file in file.name.to_vec() {
                if !path_search(cwd, repo_root, direction, &file) {
                    return false;
                }
            }
        }

        true
    }
}

#[derive(Debug, Deserialize)]
struct Config {
    #[serde(rename = "entry")]
    entries: Vec<Entry>,
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
    let Ok(command) = Command::from_str(&program_name) else {
        error!(
            "cbtr multicall program invoked with unexpected name '{}'. Valid options are {}",
            program_name, options
        );
        std::process::exit(2);
    };

    let Some(proj_dirs) = ProjectDirs::from("", "", "cbtr") else {
        error!("Couldn't find proj dirs");
        std::process::exit(1);
    };

    // TODO: config dir should contain multiple tomls, where each toml could share the same 'entry.file' or 'entry.bin'
    let config_dir = proj_dirs.config_dir();
    if !config_dir.is_dir() {
        fs::create_dir_all(&config_dir)?;
    }

    let config_file = config_dir.join("config.toml");
    if !config_file.is_file() {
        error!("Please create a cbtr config at {}", config_file.display());
        std::process::exit(1);
    };

    let contents = fs::read_to_string(&config_file)?;
    let config: Config = toml::from_str(&contents)?;

    let cwd = env::current_dir()?;
    let root = match repo_root(&cwd) {
        Ok(root) => root,
        Err(_) => {
            // Fall back to cwd if we aren't working in a git repo
            warn!("Current dir is not within a git repo. Using CWD as repo root");
            cwd.clone()
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
