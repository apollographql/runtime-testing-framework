//! Providers are how we expose the rest of the framework to user facing config.

use std::path::PathBuf;

pub(crate) mod file;
pub(crate) struct Context {
    pub(crate) config_dir: PathBuf,
}

impl Context {
    pub(crate) fn new(config_dir: impl Into<PathBuf>) -> Self {
        Self {
            config_dir: config_dir.into(),
        }
    }
}
