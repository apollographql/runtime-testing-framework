use std::{
    collections::HashMap,
    env::set_current_dir,
    fs, io,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathKind {
    Missing,
    File,
    EmptyDir,
    OccupiedDir,
}

/// Types that implement ResolutionContext may be used to perform IO while validating and resolving
/// providers.
///
/// This trait is used to allow us to inject mock IO implementations in tests so we do not need to
/// make use of temp directories or mock servers. For the most part the methods provided by this
/// trait are wrappers around [std::fs] and API clients for making requests to third party APIs.
///
/// For the canonical real implementations that should be mocked see [Context].
pub trait ResolutionContext {
    /// Returns the canonical, absolute form of the path with all intermediate
    /// components normalized and symbolic links resolved.
    fn canonicalize_path(&self, relative_path: impl AsRef<Path>) -> io::Result<PathBuf>;

    fn dir_containing(&self, path: impl AsRef<Path>) -> PathBuf {
        match path.as_ref().parent() {
            Some(p) => p.to_path_buf(),
            None => PathBuf::new(),
        }
    }

    /// Categorise the provided path.
    ///
    /// If you cannot access the metadata of the file, e.g. because of a permission error or broken
    /// symbolic links, this will return [PathKind::Missing].
    fn path_kind(&self, path: impl AsRef<Path>) -> PathKind;

    /// Reads the entire contents of a file into a string.
    ///
    /// # Errors
    ///
    /// This function will return an error if `path` does not already exist. Other errors may also
    /// be returned according to [OpenOptions::open][0].
    ///
    /// If the contents of the file are not valid UTF-8, then an error will also be returned.
    ///
    /// While reading from the file, this function handles [io::ErrorKind::Interrupted][1] with
    /// automatic retries. See [io::Read][2] documentation for details.
    ///
    /// [0]: std::fs::OpenOptions::open
    /// [1]: std::io::ErrorKind::Interrupted
    /// [2]: std::io::Read
    fn read_path_to_string(&self, path: impl AsRef<Path>) -> io::Result<String>;

    /// Writes a slice as the entire contents of a file.
    ///
    /// This function will create a file if it does not exist, and will entirely replace its
    /// contents if it does.
    ///
    /// Depending on the platform, this function may fail if the full directory path does not
    /// exist.
    fn write(&self, path: impl AsRef<Path>, content: impl AsRef<[u8]>) -> io::Result<()>;

    /// Spawn the specified program as a subprocess with the provided arguments.
    /// This method will block until the process completes and return the stdout of the process as
    /// a utf-8 string.
    fn run_command_blocking(
        &self,
        prog: &str,
        args: &[&str],
        env_vars: &HashMap<String, String>,
    ) -> io::Result<String>;

    /// Changes the current working directory to the specified path.
    fn set_current_dir(&mut self, path: impl AsRef<Path>) -> io::Result<()>;

    /// Recursively create a directory and all of its parent components if they
    /// are missing.
    ///
    /// If this function returns an error, some of the parent components might have
    /// been created already.
    ///
    /// If the empty path is passed to this function, it always succeeds without
    /// creating any directories.
    fn create_dir_all(&self, path: impl AsRef<Path>) -> io::Result<()>;
}

/// A [ResolutionContext] that will perform real IO.
#[derive(Debug)]
pub struct Context {
    pub(crate) cwd: PathBuf,
}

impl Context {
    /// Construct a new `Context` which will resolve paths relative to the provided directory.
    pub fn new(cwd: impl Into<PathBuf>) -> Self {
        Self { cwd: cwd.into() }
    }
}

impl ResolutionContext for Context {
    fn canonicalize_path(&self, relative_path: impl AsRef<Path>) -> io::Result<PathBuf> {
        relative_path.as_ref().canonicalize()
    }

    fn path_kind(&self, path: impl AsRef<Path>) -> PathKind {
        let p = path.as_ref();

        if !p.exists() {
            return PathKind::Missing;
        } else if p.is_file() {
            return PathKind::File;
        }

        match p.read_dir() {
            Ok(mut rd) => {
                if rd.next().is_some() {
                    PathKind::OccupiedDir
                } else {
                    PathKind::EmptyDir
                }
            }
            Err(_) => PathKind::Missing,
        }
    }

    fn read_path_to_string(&self, path: impl AsRef<Path>) -> io::Result<String> {
        fs::read_to_string(path)
    }

    fn write(&self, path: impl AsRef<Path>, contents: impl AsRef<[u8]>) -> io::Result<()> {
        fs::write(path, contents)
    }

    fn run_command_blocking(
        &self,
        prog: &str,
        args: &[&str],
        env_vars: &HashMap<String, String>,
    ) -> io::Result<String> {
        let output = Command::new(prog)
            .args(args)
            .envs(env_vars)
            .stdout(Stdio::piped())
            .output()?;

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    fn set_current_dir(&mut self, path: impl AsRef<Path>) -> io::Result<()> {
        set_current_dir(path.as_ref())?;
        self.cwd = path.as_ref().to_path_buf();

        Ok(())
    }

    fn create_dir_all(&self, path: impl AsRef<Path>) -> io::Result<()> {
        fs::create_dir_all(path)
    }
}
