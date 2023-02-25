use anyhow::{bail, Result};
use clap::Parser;
use commandstream::CommandStream;
use directories::{BaseDirs, ProjectDirs, UserDirs};
use gix::Repository;
use log::{debug, info};
use serde::{Deserialize, Serialize};
use std::env::{self, current_dir};
use std::io::stdout;
use std::io::Write;
use std::path::{Path, PathBuf};

const PACKAGE_NAME: &str = env!("CARGO_PKG_NAME");
const DEFAULT_INDENT: &str = "  | ";

#[derive(Parser, Debug)]
struct Args {
    /// Config file path
    #[arg(long)]
    config_file: Option<PathBuf>,
}

#[derive(Debug, Eq, PartialEq, Deserialize, Serialize)]
struct Settings {
    indent: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            indent: Some(String::from(DEFAULT_INDENT)),
        }
    }
}

#[derive(Debug, Eq, PartialEq, Deserialize, Serialize)]
struct Config {
    settings: Option<Settings>,
    targets: Vec<Target>,
}

#[derive(Debug, Eq, PartialEq, Deserialize, Serialize)]
struct Target {
    name: String,
    detect: Vec<String>,
    check: Option<Vec<String>>,
    build: Option<Vec<String>>,
    test: Option<Vec<String>>,
    run: Option<Vec<String>>,
}

fn nammed_file_exists(file_name: &str, current_dir: &Path, repo_root: &Path) -> Option<PathBuf> {
    // Keep stepping back a directory checking if the file exists
    let mut current_dir = current_dir.to_path_buf();
    loop {
        let mut possible = current_dir.to_path_buf();
        possible.push(file_name);
        if let Ok(true) = possible.try_exists() {
            return Some(possible);
        }

        current_dir.pop();

        // If the newly-popped current working dir is not a subdir of the git repo, we cannot check any further back
        if !current_dir.starts_with(repo_root) {
            return None;
        }
    }
}

#[derive(Debug)]
enum Operation {
    Check,
    Build,
    Test,
    Run,
}

struct Command<'a> {
    indent: &'a str,
    command: &'a [String],
}
impl<'a> Command<'a> {
    fn new(command: &'a [String], indent: &'a str) -> Self {
        Command { indent, command }
    }
}
impl<'a> CommandStream<'_> for Command<'a> {
    fn command(&self) -> &[String] {
        &self.command
    }
    fn handle_stdout(&self, line: &str) -> Result<()> {
        println!("{}{}", self.indent, line);
        Ok(())
    }
    fn handle_stderr(&self, line: &str) -> Result<()> {
        eprintln!("{}{}", self.indent, line);
        Ok(())
    }
}

pub async fn run() -> Result<()> {
    env_logger::init();
    let args = Args::parse();

    let mut config_file = match args.config_file {
        Some(config_file) => config_file,
        None => {
            let Some(project_dirs) = ProjectDirs::from("", "", PACKAGE_NAME) else {
                bail!("todo error message 2");
            };
            let mut config_file = PathBuf::from(project_dirs.config_dir());
            config_file.push("config.yaml");
            config_file
        }
    };
    match config_file.canonicalize() {
        Ok(cannonicalized) => {
            config_file = cannonicalized;
        }
        Err(e) => {
            bail!("Unable to read config file: {}", e);
        }
    };

    debug!("Config file: {}", config_file.display());
    let f = std::fs::File::open(config_file)?;
    let config: Config = serde_yaml::from_reader(f)?;
    let settings = match config.settings {
        Some(settings) => settings,
        None => Settings::default(),
    };
    let indent = match settings.indent {
        Some(indent) => indent,
        None => String::from(DEFAULT_INDENT),
    };

    let Ok(exec_path) = env::current_exe() else {
        bail!("todo error message");
    };
    let exec_name = exec_path.file_name().unwrap().to_str().unwrap();
    let operation = match exec_name {
        "c" => Operation::Check,
        "b" => Operation::Build,
        "t" => Operation::Test,
        "r" => Operation::Run,
        _ => bail!("Unable to determine operation from this exec's name"),
    };

    let current_working_dir = env::current_dir()?;
    let repo = gix::discover(current_working_dir.clone())?;
    let repo_root = gix::discover::path::without_dot_git_dir(repo.path().to_path_buf());

    for target in config.targets {
        let mut found_all_requirements = true;
        for required_file in target.detect {
            if let Some(found_file_path) =
                nammed_file_exists(&required_file, &current_working_dir, &repo_root)
            {
                debug!("Found {}", found_file_path.display());
            } else {
                // Any missing file means we skip this target
                debug!("Unable to find {}", required_file);
                found_all_requirements = false;
                break;
            }
        }
        if found_all_requirements {
            let commands = match operation {
                Operation::Check => target.check,
                Operation::Build => target.build,
                Operation::Test => target.test,
                Operation::Run => target.run,
            };

            let Some(commands) = commands else {
              bail!("No registered {:?} operation for target '{}'", operation, target.name)
            };
            for c in commands {
                println!("Running {:?}", c);
                stdout().flush()?; // Need to flush before running command or the last print could
                                   // get out of order
                let args: Vec<String> = c.split_whitespace().map(|x| x.to_owned()).collect();
                let cmd = Command::new(&args, &indent);
                let return_code = cmd.run().await?;
                println!("command exited with return code {}", return_code)
            }

            break; // Skip any remaining targets. Order matters
        }
    }
    Ok(())
}
