//! Run validation tests for a custom provider
use crate::commands::{get_context, plumbing::custom_provider::load_definition};
use anyhow::{Context, anyhow};
use assert_fs::TempDir;
use rtf_config::{
    Source,
    checks::Check,
    context::ResolutionContext,
    formats::CustomProviderDefinition,
    templating::{Scalar, Template, TemplateContext},
};
use std::{
    collections::HashMap,
    fs, io,
    path::{Path, PathBuf},
};
use tracing::{error, info};

const EXPECTED_RUN_ERROR_FILE: &str = "expected-run-error.txt";
const EXPECTED_RUN_OUTPUT_DIR: &str = "expected-run-output";
const VARIABLES_FILE: &str = "variables.json";

pub async fn test_custom_provider(
    definition_path: &str,
    test_cases_dir: &str,
    error_on_empty: bool,
) -> anyhow::Result<()> {
    let mut ctx = get_context();

    let (source, definition) = load_definition(definition_path, &ctx).await?;
    let test_cases = TestCase::try_load_all(&ctx.canonicalize_path(test_cases_dir)?)?;

    if test_cases.is_empty() && error_on_empty {
        error!("No test cases found");
        return Err(anyhow!("No test cases found"));
    }

    let mut failures = Vec::new();
    for case in test_cases.into_iter() {
        if let Err(err) = case.run(&source, definition.clone(), &mut ctx).await {
            failures.push(err);
        }
    }

    if !failures.is_empty() {
        todo!("show failures")
    }

    Ok(())
}

#[derive(Debug)]
struct TestCase {
    path: PathBuf,
    expect_failure: bool,
}

impl TestCase {
    fn try_load_all(dir: &Path) -> anyhow::Result<Vec<Self>> {
        let io_err =
            |kind: io::ErrorKind, p: &Path| io::Error::new(kind, p.display().to_string()).into();

        let mut test_cases = Vec::new();

        if !dir.exists() {
            return Err(io_err(io::ErrorKind::NotFound, dir));
        } else if !dir.is_dir() {
            return Err(io_err(io::ErrorKind::NotADirectory, dir));
        }

        for entry in dir.read_dir()? {
            let entry = entry?;
            let path = entry.path();
            if !path.is_dir() {
                return Err(io_err(io::ErrorKind::NotADirectory, dir));
            }

            let vars_file = path.join(VARIABLES_FILE);
            if !vars_file.exists() {
                return Err(io_err(io::ErrorKind::NotFound, &vars_file));
            } else if vars_file.is_dir() {
                return Err(io_err(io::ErrorKind::IsADirectory, &vars_file));
            }

            let expect_failure = path.join(EXPECTED_RUN_ERROR_FILE).exists();
            let expected_output_dir = path.join(EXPECTED_RUN_OUTPUT_DIR);
            if !expect_failure {
                if !expected_output_dir.exists() {
                    return Err(io_err(io::ErrorKind::NotFound, &vars_file));
                } else if !expected_output_dir.is_dir() {
                    return Err(io_err(io::ErrorKind::NotADirectory, &expected_output_dir));
                }
            }

            test_cases.push(TestCase {
                path,
                expect_failure,
            });
        }

        Ok(test_cases)
    }

    async fn run(
        self,
        source: &Source,
        mut definition: CustomProviderDefinition,
        ctx: &mut impl ResolutionContext,
    ) -> anyhow::Result<Option<Failure>> {
        let template_ctx = TemplateContext::new(
            load_variables(&self.path.join(VARIABLES_FILE))?,
            source.clone(),
            Default::default(),
            Default::default(),
        );

        if let Err(e) = definition.try_template(&mut Vec::new(), source, &template_ctx) {
            return Ok(Some(Failure::Template { err: e.to_string() }));
        } else if let Err(e) = definition.command.try_check(&mut Vec::new(), ctx) {
            return Ok(Some(Failure::Check { err: e.to_string() }));
        }

        // We run the provider in an self-removing temp directory so we don't need to worry about
        // manual cleanup of test data.
        let tmp_dir = TempDir::new()?;

        if let Source::Local { abs_path } = &source {
            let definition_dir = ctx.dir_containing(abs_path);
            ctx.set_current_dir(definition_dir)?;
        }

        definition
            .command
            .run_providers_and_execute(tmp_dir.path(), None, ctx)
            .await?;

        Ok(None)
    }
}

/// Failure reasons from running the test case that relate to the structure or behaviour of the
/// custom provider itself.
///
/// Failures from IO around setting up and running the test itself are reported separately
enum Failure {
    Template {
        err: String,
    },

    Check {
        err: String,
    },

    Run {
        err: String,
    },

    ExpectedError {
        expected: String,
        actual: String,
    },

    OutputDiff {
        missing: Vec<PathBuf>,
        unexpected: Vec<PathBuf>,
        with_diff: Vec<(PathBuf, String)>,
    },
}

fn load_variables(path: &Path) -> anyhow::Result<HashMap<String, Scalar>> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("Failed to read variables file: {}", path.display()))?;
    let vars: HashMap<String, Scalar> = serde_json::from_str(&content)
        .with_context(|| format!("Failed to parse variables JSON: {}", path.display()))?;

    Ok(vars)
}
