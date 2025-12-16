//! Metadata structs used for tracking where individual configuration files have been sourced from.
//! This is used to support the behaviour of the RelativeFile file provider.
use crate::{
    context::ResolutionContext,
    providers::{self, Result},
};
use rtf_integrations::github::Client;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{
    fmt, io,
    path::{Path, PathBuf},
};

/// The source directory of where a particular config file was obtained.
///
/// In its simplest form this is a local file path to the directory containing the config file, but
/// this may also include things like pulling the file over the network.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum SourceDir {
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

impl SourceDir {
    pub fn local(abs_path: impl Into<PathBuf>) -> Self {
        Self::Local {
            abs_path: abs_path.into(),
        }
    }

    pub fn github(
        org: impl Into<String>,
        repo: impl Into<String>,
        path: impl Into<PathBuf>,
        git_ref: Option<impl Into<String>>,
    ) -> Self {
        Self::Github {
            org: org.into(),
            repo: repo.into(),
            path: path.into(),
            git_ref: git_ref.map(Into::into),
        }
    }

    pub(crate) fn to_uri_for(&self, child_path: impl AsRef<Path>) -> String {
        self.to_uri(Some(child_path))
    }

    fn to_uri(&self, child_path: Option<impl AsRef<Path>>) -> String {
        let full_path = |base: &Path| match child_path {
            Some(tail) => base.join(tail),
            None => base.to_path_buf(),
        };

        match self {
            Self::Local { abs_path } => {
                format!("file://{}", full_path(abs_path).display())
            }

            Self::Github {
                org,
                repo,
                path,
                git_ref: Some(git_ref),
            } => format!(
                "https://github.com/{org}/{repo}/{}?ref={git_ref}",
                full_path(path).display()
            ),

            Self::Github {
                org,
                repo,
                path,
                git_ref: None,
            } => format!(
                "https://github.com/{org}/{repo}/{}",
                full_path(path).display()
            ),
        }
    }

    pub async fn try_get_file_content(
        &self,
        fname: impl AsRef<Path>,
        ctx: &impl ResolutionContext,
    ) -> Result<String> {
        match self {
            Self::Local { abs_path } => Ok(ctx.read_path_to_string(abs_path.join(fname))?),
            Self::Github {
                org,
                repo,
                path,
                git_ref,
            } => {
                let client = ctx.github_client().ok_or(providers::Error::Github(
                    rtf_integrations::github::Error::NoClient,
                ))?;

                Ok(client
                    .string_file_content(
                        org,
                        repo,
                        &path.join(fname).display().to_string(),
                        git_ref.as_ref(),
                    )
                    .await?)
            }
        }
    }
}

impl Default for SourceDir {
    fn default() -> Self {
        Self::Local {
            abs_path: PathBuf::new(),
        }
    }
}

impl fmt::Display for SourceDir {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_uri(Option::<&str>::None))
    }
}

/// # Config Source
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum RawSource {
    Local {
        /// A relative path to the target config file
        relative_path: PathBuf,
    },
    Github {
        /// The GitHub org to pull the config file from
        org: String,
        /// The GitHub repository to pull the config file from
        repo: String,
        /// The absolute path to the config file within the target GitHub repository
        path: PathBuf,
        /// An optional git ref to pull the config file from (defaults to mainline)
        #[serde(default)]
        git_ref: Option<String>,
    },
}

impl RawSource {
    pub fn try_into_source_and_filename(
        self,
        file_source: &SourceDir,
        ctx: &impl ResolutionContext,
    ) -> io::Result<(SourceDir, String)> {
        let (mut src, file_name) = self.try_into_source_without_canonical_path(file_source)?;
        if let SourceDir::Local { abs_path } = &mut src {
            *abs_path = ctx.canonicalize_path(&*abs_path)?;
        }

        Ok((src, file_name))
    }

    pub(crate) fn with_child_path(&self, child_path: impl AsRef<Path>) -> Self {
        let mut new = self.clone();
        match &mut new {
            Self::Local { relative_path } => *relative_path = relative_path.join(child_path),
            Self::Github { path, .. } => *path = path.join(child_path),
        }

        new
    }

    fn try_into_source_without_canonical_path(
        self,
        file_source: &SourceDir,
    ) -> io::Result<(SourceDir, String)> {
        let split = |p: PathBuf| {
            let file_name = p
                .file_name()
                .and_then(|os_str| os_str.to_str())
                .ok_or_else(|| io::Error::other("expected a filename"))?
                .to_string();
            let p = p.parent().unwrap().to_owned();

            io::Result::Ok((p, file_name))
        };

        match self {
            Self::Local { relative_path } => {
                let (relative_dir, file_name) = split(relative_path)?;

                let src = match file_source {
                    SourceDir::Local {
                        abs_path: containing_dir,
                    } => SourceDir::Local {
                        abs_path: containing_dir.join(relative_dir),
                    },

                    SourceDir::Github {
                        org,
                        repo,
                        path: containing_dir,
                        git_ref,
                    } => SourceDir::Github {
                        org: org.clone(),
                        repo: repo.clone(),
                        path: containing_dir.join(relative_dir),
                        git_ref: git_ref.clone(),
                    },
                };

                Ok((src, file_name))
            }

            Self::Github {
                org,
                repo,
                path,
                git_ref,
            } => {
                let (path, file_name) = split(path)?;

                let src = SourceDir::Github {
                    org,
                    repo,
                    path,
                    git_ref,
                };

                Ok((src, file_name))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use simple_test_case::test_case;

    fn raw_local(path: &str) -> RawSource {
        RawSource::Local {
            relative_path: path.into(),
        }
    }

    fn raw_gh(org: &str, repo: &str, path: &str, git_ref: Option<&str>) -> RawSource {
        RawSource::Github {
            org: org.into(),
            repo: repo.into(),
            path: path.into(),
            git_ref: git_ref.map(Into::into),
        }
    }

    // Option<impl Into<String>> requires explicit typing for "None"
    const STR_NONE: Option<&str> = None;

    // Github RawSource is independent of the test plan source so these should all just map
    // directly from their raw to "cooked" counterpart
    #[test_case(
        raw_gh("org", "repo", "bar/environment.yaml", STR_NONE),
        SourceDir::local("foo"),
        SourceDir::github("org", "repo", "bar", STR_NONE);
        "local test plan github raw"
    )]
    #[test_case(
        raw_gh("org", "repo", "bar/environment.yaml", Some("branch")),
        SourceDir::local("foo"),
        SourceDir::github("org", "repo", "bar", Some("branch"));
        "local test plan github raw with branch"
    )]
    #[test_case(
        raw_gh("org", "repo", "bar/environment.yaml", STR_NONE),
        SourceDir::github("org", "repo", "foo", STR_NONE),
        SourceDir::github("org", "repo", "bar", STR_NONE);
        "github test plan github raw"
    )]
    #[test_case(
        raw_gh("org", "repo", "bar/environment.yaml", Some("branch")),
        SourceDir::github("org", "repo", "foo", Some("branch")),
        SourceDir::github("org", "repo", "bar", Some("branch"));
        "github test plan github raw with branch"
    )]
    #[test_case(
        raw_gh("org", "repo", "bar/environment.yaml", Some("branch")),
        SourceDir::github("org", "repo", "foo", STR_NONE),
        SourceDir::github("org", "repo", "bar", Some("branch"));
        "github test plan without branch github raw with branch"
    )]
    #[test_case(
        raw_gh("org", "repo", "bar/environment.yaml", Some("branch")),
        SourceDir::github("org", "repo", "foo", Some("other-branch")),
        SourceDir::github("org", "repo", "bar", Some("branch"));
        "github test plan with different branch github raw with branch"
    )]
    // Local TP + local raw should update the relative path based on the directory containing the
    // test plan
    #[test_case(
        raw_local("bar/environment.yaml"),
        SourceDir::local("foo"),
        SourceDir::local("foo/bar");
        "local test plan local raw"
    )]
    // Github test plan source should rewrite local raw sources to be github sources as well
    #[test_case(
        raw_local("bar/environment.yaml"),
        SourceDir::github("org", "repo", "foo", STR_NONE),
        SourceDir::github("org", "repo", "foo/bar", STR_NONE);
        "github test plan local raw"
    )]
    #[test_case(
        raw_local("bar/environment.yaml"),
        SourceDir::github("org", "repo", "foo", Some("branch")),
        SourceDir::github("org", "repo", "foo/bar", Some("branch"));
        "github test plan with branch local raw"
    )]
    #[test]
    fn raw_source_try_into_source_respects_parent_source_kind(
        raw: RawSource,
        tp_source: SourceDir,
        expected: SourceDir,
    ) {
        let (src, _file_name) = raw
            .try_into_source_without_canonical_path(&tp_source)
            .unwrap();

        assert_eq!(src, expected);
    }
}
