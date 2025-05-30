use std::{
    collections::HashMap,
    fs, io,
    path::{Path, PathBuf},
    process::{Command, ExitStatus},
};

/// Types that implement ResolutionContext may be used to perform IO while validating and resolving
/// providers.
///
/// This trait is used to allow us to inject mock IO implementations in tests so we do not need to
/// make use of temp directories or mock servers. For the most part the methods provided by this
/// trait are wrappers around [std::fs] and API clients for making requests to third party APIs.
///
/// For the canonical real implementations that should be mocked see [Context].
pub trait ResolutionContext {
    /// Attempt to resolve a path relative to the directory containing the config file being
    /// processed.
    fn resolve_path(&self, relative_path: impl AsRef<Path>) -> io::Result<PathBuf>;

    /// Returns `true` if the path points at an existing filesystem entity.
    ///
    /// If you cannot access the metadata of the file, e.g. because of a permission error or broken
    /// symbolic links, this will return `false`.
    fn path_exists(&self, path: impl AsRef<Path>) -> bool;

    /// Returns `true` if the path exists on disk and is pointing at a regular file.
    ///
    /// If you cannot access the metadata of the file, e.g. because of a permission error or broken
    /// symbolic links, this will return `false`.
    fn path_is_file(&self, path: impl AsRef<Path>) -> bool;

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
    /// This method will block until the process completes and return the status code. Stdout and
    /// Stderr will be inherited from the current process.
    fn run_command_blocking(
        &self,
        prog: &str,
        args: &[&str],
        env_vars: &HashMap<String, String>,
    ) -> io::Result<ExitStatus>;
}

/// A [ResolutionContext] that will perform real IO.
#[derive(Debug)]
pub struct Context {
    pub(crate) config_dir: PathBuf,
}

impl Context {
    /// Construct a new `Context` which will resolve paths relative to the provided directory.
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

    fn run_command_blocking(
        &self,
        prog: &str,
        args: &[&str],
        env_vars: &HashMap<String, String>,
    ) -> io::Result<ExitStatus> {
        let mut child = Command::new(prog).args(args).envs(env_vars).spawn()?;

        child.wait()
    }
}
