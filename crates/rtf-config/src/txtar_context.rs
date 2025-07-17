use crate::context::{PathKind, ResolutionContext};
use rtf_core::graphos::platform_query;
use rtf_core::{HttpClient, github};
use simple_txtar::Archive;
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};

pub struct TxtarContext {
    pub arr: Archive,
}

impl ResolutionContext for TxtarContext {
    type PlatformClient = NullClient;
    type GithubClient = NullClient;
    type HttpClient = NullClient;

    fn run_command_blocking<'a>(
        &self,
        _prog: &str,
        _args: impl IntoIterator<Item = &'a str>,
        _env_vars: &HashMap<String, String>,
    ) -> io::Result<()> {
        Ok(())
    }

    fn write(&self, _path: impl AsRef<Path>, _content: impl AsRef<[u8]>) -> io::Result<()> {
        unimplemented!()
    }

    fn path_kind(&self, _path: impl AsRef<Path>) -> crate::context::PathKind {
        PathKind::File
    }

    fn canonicalize_path(&self, relative_path: impl AsRef<Path>) -> io::Result<PathBuf> {
        Ok(relative_path.as_ref().to_path_buf())
    }

    fn read_path_to_string(&self, path: impl AsRef<Path>) -> io::Result<String> {
        let p = path.as_ref().display().to_string();

        self.arr
            .get(&p)
            .map(|f| f.content.clone())
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, ""))
    }

    fn remove_file(&self, _path: impl AsRef<Path>) -> io::Result<()> {
        Ok(())
    }

    fn set_current_dir(&mut self, _path: impl AsRef<Path>) -> io::Result<()> {
        Ok(())
    }

    fn create_dir_all(&self, _path: impl AsRef<Path>) -> io::Result<()> {
        Ok(())
    }
}

/// Used to implement [ResolutionContext] in tests where no client is needed.
#[cfg(test)]
#[derive(Debug, Clone, Copy)]
pub struct NullClient;

#[cfg(test)]
impl platform_query::Client for NullClient {
    async fn post_operation(
        &self,
        _body: &impl serde::Serialize,
    ) -> Result<serde_json::Value, platform_query::Error> {
        panic!("a NullClient can not be used to make requests")
    }
}

#[cfg(test)]
impl github::Client for NullClient {
    async fn raw_file_content(
        &self,
        _org: &str,
        _repo: &str,
        _path: &str,
        _git_ref: Option<impl AsRef<str>>,
    ) -> Result<bytes::Bytes, github::Error> {
        panic!("a NullClient can not be used to make requests")
    }
}

#[cfg(test)]
impl HttpClient for NullClient {
    async fn get(&self, _url: &str) -> Result<rtf_core::HttpResponse, reqwest::Error> {
        panic!("a NullClient can not be used to make requests")
    }
}
