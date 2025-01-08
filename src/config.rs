use log::debug;
use pathdiff::diff_paths;
use serde::Deserialize;
use std::path::Component;
use std::path::Path;

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

#[derive(Deserialize, Debug)]
#[serde(untagged)]
pub enum StringOrVec {
    Single(String),
    Multiple(Vec<String>),
}

impl StringOrVec {
    pub fn to_vec(&self) -> Vec<String> {
        match self {
            StringOrVec::Single(s) => vec![s.clone()],
            StringOrVec::Multiple(v) => v.clone(),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct Tools {
    pub format: Option<StringOrVec>,
    pub check: Option<StringOrVec>,
    pub build: Option<StringOrVec>,
    pub test: Option<StringOrVec>,
    pub run: Option<StringOrVec>,
}

#[derive(Debug, Deserialize, Default)]
pub enum Direction {
    #[serde(rename = "backwards")]
    Backwards,
    #[default]
    #[serde(rename = "forwards")]
    Forwards,
}

#[derive(Debug, Deserialize)]
pub struct File {
    pub(crate) name: StringOrVec,
    #[serde(rename = "search-direction", default)]
    search_direction: Direction,
}

#[derive(Debug, Deserialize)]
pub struct Entry {
    pub(crate) name: String,
    pub(crate) bin: Option<StringOrVec>,
    pub(crate) file: Option<File>,
    pub(crate) tools: Tools,
}

impl Entry {
    pub fn matches(&self, cwd: &Path, repo_root: &Path) -> bool {
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
pub struct Config {
    #[serde(rename = "entry")]
    pub(crate) entries: Vec<Entry>,
}

impl Config {
    pub fn append(&mut self, mut other: Config) {
        self.entries.append(&mut other.entries);
    }
}
