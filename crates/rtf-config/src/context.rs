use std::{
    fs, io,
    path::{Path, PathBuf},
};

pub trait ResolutionContext {
    fn resolve_path(&self, relative_path: impl AsRef<Path>) -> io::Result<PathBuf>;
    fn path_exists(&self, path: impl AsRef<Path>) -> bool;
    fn path_is_file(&self, path: impl AsRef<Path>) -> bool;
    fn read_path_to_string(&self, path: impl AsRef<Path>) -> io::Result<String>;
    fn write(&self, path: impl AsRef<Path>, content: impl AsRef<[u8]>) -> io::Result<()>;
}

#[derive(Debug)]
pub struct Context {
    pub(crate) config_dir: PathBuf,
}

impl Context {
    pub fn new(config_dir: impl Into<PathBuf>) -> Self {
        Self {
            config_dir: config_dir.into(),
        }
    }
}

impl ResolutionContext for Context {
    fn resolve_path(&self, relative_path: impl AsRef<Path>) -> io::Result<PathBuf> {
        self.config_dir.join(relative_path).canonicalize()
    }

    fn path_exists(&self, path: impl AsRef<Path>) -> bool {
        path.as_ref().exists()
    }

    fn path_is_file(&self, path: impl AsRef<Path>) -> bool {
        path.as_ref().is_file()
    }

    fn read_path_to_string(&self, path: impl AsRef<Path>) -> io::Result<String> {
        fs::read_to_string(path)
    }

    fn write(&self, path: impl AsRef<Path>, contents: impl AsRef<[u8]>) -> io::Result<()> {
        fs::write(path, contents)
    }
}
