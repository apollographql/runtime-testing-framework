//! Metadata structs used for tracking where individual configuration files have been sourced from.
//! This is used to support the behaviour of the RelativeFile file provider.
use crate::{
    context::ResolutionContext,
    providers::{self, Result},
};
use rtf_core::github::Client;
use serde::{Deserialize, Serialize};
use std::{io, path::PathBuf};

/// The source of how a particular config file was obtained.
///
/// In its simplest form this is a local file path to the directory containing the config file, but
/// this may also include things like pulling the file over the network.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Source {
    /// The config file was read from disk
    Local {
        /// The absolute path to the config file
        abs_path: PathBuf,
    },

    /// The config file was downloaded from GitHub
    Github {
        /// The GitHub org
        org: String,
        /// The GitHub repository
        repo: String,
        /// The path to the file within the GitHub repository
        path: PathBuf,
        /// An optional ref of the repo to use (the default branch is used when None)
        git_ref: Option<String>,
    },
}

impl Source {
    pub fn local(abs_path: impl Into<PathBuf>) -> Self {
        Self::Local {
            abs_path: abs_path.into(),
        }
    }

    pub async fn try_get_file_content(&self, ctx: &impl ResolutionContext) -> Result<String> {
        match self {
            Self::Local { abs_path } => Ok(ctx.read_path_to_string(abs_path)?),
            Self::Github {
                org,
                repo,
                path,
                git_ref,
            } => {
                let client = ctx
                    .github_client()
                    .ok_or(providers::Error::Github(rtf_core::github::Error::NoClient))?;

                Ok(client
                    .string_file_content(org, repo, &path.display().to_string(), git_ref.as_ref())
                    .await?)
            }
        }
    }
}

impl Default for Source {
    fn default() -> Self {
        Self::Local {
            abs_path: PathBuf::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum RawSource {
    Local {
        relative_path: PathBuf,
    },
    Github {
        org: String,
        repo: String,
        path: String,
        #[serde(default)]
        git_ref: Option<String>,
    },
}

impl RawSource {
    pub fn try_into_source(
        self,
        tp_source: &Source,
        ctx: &impl ResolutionContext,
    ) -> io::Result<Source> {
        match self {
            Self::Local { relative_path } => match tp_source {
                Source::Local { abs_path } => {
                    let p = match abs_path.parent() {
                        Some(parent) => parent.join(relative_path),
                        None => relative_path,
                    };

                    Ok(Source::Local {
                        abs_path: ctx.canonicalize_path(p)?,
                    })
                }

                Source::Github {
                    org,
                    repo,
                    path,
                    git_ref,
                } => {
                    let path = match path.parent() {
                        Some(parent) => parent.join(relative_path),
                        None => relative_path,
                    };

                    Ok(Source::Github {
                        org: org.clone(),
                        repo: repo.clone(),
                        path,
                        git_ref: git_ref.clone(),
                    })
                }
            },

            Self::Github {
                org,
                repo,
                path,
                git_ref,
            } => Ok(Source::Github {
                org,
                repo,
                path: PathBuf::from(path),
                git_ref,
            }),
        }
    }
}
