use rtf_core::{
    APOLLO_KEY_ENV_VAR, APOLLO_SUDO_ENV_VAR, GRAPH_OS_STAGING_ENV_VAR, ReqwestClient,
    graphos::platform_query,
};
use std::{
    collections::HashMap,
    env::set_current_dir,
    fs::{self, File},
    io,
    os::unix::fs::PermissionsExt,
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
    type PlatformClient: platform_query::Client;

    /// Provide a [Client][platform_query::Client] for making requests to the Apollo platform API.
    ///
    /// If it is not possible for this current context to make requests to the platform API then
    /// this method should return [None].
    fn platform_client(&self) -> Option<&Self::PlatformClient> {
        None
    }

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

    /// Changes the permissions of the specified file
    fn make_executable(&self, path: impl AsRef<Path>) -> io::Result<()> {
        let file = File::open(path.as_ref())?;
        let mut permissions = file.metadata()?.permissions();
        permissions.set_mode(0o777);

        file.set_permissions(permissions)
    }

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
#[derive(Default, Debug)]
pub struct Context {
    pub(crate) client: ReqwestClient,
}

impl Context {
    /// Construct a new `Context` with default configuration.
    pub fn new() -> Self {
        Self {
            client: ReqwestClient::default(),
        }
    }

    /// Construct a new `Context` with environment variables.
    pub fn new_from_env_vars(mut env_vars: HashMap<String, String>) -> Self {
        let mut ctx = Self::new();

        if let Some(api_key) = env_vars.remove(APOLLO_KEY_ENV_VAR) {
            let staging = matches!(
                env_vars.remove(GRAPH_OS_STAGING_ENV_VAR).as_deref(),
                Some("true" | "1")
            );
            let sudo = matches!(
                env_vars.remove(APOLLO_SUDO_ENV_VAR).as_deref(),
                Some("true" | "1")
            );

            ctx.with_platform_config(api_key, staging, sudo);
        }

        ctx
    }

    /// Provide configuration for making requests to the Apollo platform API.
    pub fn with_platform_config(
        &mut self,
        api_key: impl Into<String>,
        staging: bool,
        sudo: bool,
    ) -> &mut Self {
        self.client.with_platform_config(api_key, staging, sudo);
        self
    }
}

impl ResolutionContext for Context {
    type PlatformClient = rtf_core::graphos::PlatformClient;

    fn platform_client(&self) -> Option<&Self::PlatformClient> {
        self.client.platform_client()
    }

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

        Ok(())
    }

    fn create_dir_all(&self, path: impl AsRef<Path>) -> io::Result<()> {
        fs::create_dir_all(path)
    }
}

/// Used to implement [ResolutionContext] in tests where no platform client is needed.
#[cfg(test)]
#[derive(Debug, Clone, Copy)]
pub(crate) struct NullPlatformClient;

#[cfg(test)]
impl platform_query::Client for NullPlatformClient {
    async fn post_operation(
        &self,
        _body: &impl serde::Serialize,
    ) -> Result<serde_json::Value, platform_query::Error> {
        panic!("a NullClient can not be used to make requests")
    }
}
