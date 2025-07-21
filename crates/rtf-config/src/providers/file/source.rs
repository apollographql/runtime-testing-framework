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
        match self.try_into_source_without_canonical_path(tp_source) {
            Source::Local { abs_path } => Ok(Source::Local {
                abs_path: ctx.canonicalize_path(abs_path)?,
            }),

            gh => Ok(gh),
        }
    }

    fn try_into_source_without_canonical_path(self, tp_source: &Source) -> Source {
        match self {
            Self::Local { relative_path } => match tp_source {
                Source::Local {
                    abs_path: test_plan_path,
                } => {
                    let abs_path = match test_plan_path.parent() {
                        Some(parent) => parent.join(relative_path),
                        None => relative_path,
                    };

                    Source::Local { abs_path }
                }

                Source::Github {
                    org,
                    repo,
                    path: test_plan_path,
                    git_ref,
                } => {
                    let path = match test_plan_path.parent() {
                        Some(parent) => parent.join(relative_path),
                        None => relative_path,
                    };

                    Source::Github {
                        org: org.clone(),
                        repo: repo.clone(),
                        path,
                        git_ref: git_ref.clone(),
                    }
                }
            },

            Self::Github {
                org,
                repo,
                path,
                git_ref,
            } => Source::Github {
                org,
                repo,
                path: PathBuf::from(path),
                git_ref,
            },
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
        Source::local("foo/test-plan.yaml"),
        Source::github("org", "repo", "bar/environment.yaml", STR_NONE);
        "local test plan github raw"
    )]
    #[test_case(
        raw_gh("org", "repo", "bar/environment.yaml", Some("branch")),
        Source::local("foo/test-plan.yaml"),
        Source::github("org", "repo", "bar/environment.yaml", Some("branch"));
        "local test plan github raw with branch"
    )]
    #[test_case(
        raw_gh("org", "repo", "bar/environment.yaml", STR_NONE),
        Source::github("org", "repo", "foo/test-plan.yaml", STR_NONE),
        Source::github("org", "repo", "bar/environment.yaml", STR_NONE);
        "github test plan github raw"
    )]
    #[test_case(
        raw_gh("org", "repo", "bar/environment.yaml", Some("branch")),
        Source::github("org", "repo", "foo/test-plan.yaml", Some("branch")),
        Source::github("org", "repo", "bar/environment.yaml", Some("branch"));
        "github test plan github raw with branch"
    )]
    #[test_case(
        raw_gh("org", "repo", "bar/environment.yaml", Some("branch")),
        Source::github("org", "repo", "foo/test-plan.yaml", STR_NONE),
        Source::github("org", "repo", "bar/environment.yaml", Some("branch"));
        "github test plan without branch github raw with branch"
    )]
    #[test_case(
        raw_gh("org", "repo", "bar/environment.yaml", Some("branch")),
        Source::github("org", "repo", "foo/test-plan.yaml", Some("other-branch")),
        Source::github("org", "repo", "bar/environment.yaml", Some("branch"));
        "github test plan with different branch github raw with branch"
    )]
    // Local TP + local raw should update the relative path based on the directory containing the
    // test plan
    #[test_case(
        raw_local("bar/environment.yaml"),
        Source::local("foo/test-plan.yaml"),
        Source::local("foo/bar/environment.yaml");
        "local test plan local raw"
    )]
    // Github test plan source should rewrite local raw sources to be github sources as well
    #[test_case(
        raw_local("bar/environment.yaml"),
        Source::github("org", "repo", "foo/test-plan.yaml", STR_NONE),
        Source::github("org", "repo", "foo/bar/environment.yaml", STR_NONE);
        "github test plan local raw"
    )]
    #[test_case(
        raw_local("bar/environment.yaml"),
        Source::github("org", "repo", "foo/test-plan.yaml", Some("branch")),
        Source::github("org", "repo", "foo/bar/environment.yaml", Some("branch"));
        "github test plan with branch local raw"
    )]
    #[test]
    fn try_into_source_respects_parent_source_kind(
        raw: RawSource,
        tp_source: Source,
        expected: Source,
    ) {
        let src = raw.try_into_source_without_canonical_path(&tp_source);
        assert_eq!(src, expected);
    }
}
