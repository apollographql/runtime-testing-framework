//! Run validation tests for a custom provider
use crate::commands::{get_context, plumbing::custom_provider::load_definition};
use anyhow::{Context, anyhow};
use assert_fs::TempDir;
use rtf_config::{
    Source,
    checks::Check,
    context::ResolutionContext,
    formats::CustomProviderDefinition,
    providers::command::{OUTPUT_PATH, PROVIDER_DIR},
    templating::{Scalar, Template, TemplateContext},
};
use similar::{ChangeTag, TextDiff};
use std::{
    collections::HashMap,
    fs, io,
    path::{Path, PathBuf},
};
use tracing::{debug, error};
use walkdir::WalkDir;

// const EXPECTED_RUN_ERROR_FILE: &str = "expected-run-error.txt";
const EXPECTED_RUN_OUTPUT_DIR: &str = "expected-run-output";
const VARIABLES_FILE: &str = "variables.json";
const TEST_CASES_DIR: &str = "test-cases";

pub async fn test_custom_provider(
    definition_path: &str,
    test_cases_dir: Option<String>,
    error_on_empty: bool,
) -> anyhow::Result<()> {
    let ctx = get_context();

    let (source, definition) = load_definition(definition_path, &ctx).await?;

    let abs_path = ctx
        .canonicalize_path(definition_path)
        .with_context(|| format!("Unable to resolve path: {definition_path}"))?;
    let test_cases_dir = test_cases_dir.unwrap_or_else(|| {
        abs_path
            .parent()
            .unwrap()
            .join(TEST_CASES_DIR)
            .display()
            .to_string()
    });

    let test_cases = TestCase::try_load_all(&ctx.canonicalize_path(test_cases_dir)?)?;

    if test_cases.is_empty() && error_on_empty {
        error!("No test cases found");
        return Err(anyhow!("No test cases found"));
    }

    let total = test_cases.len();
    let mut failures = Vec::new();
    let mut passed = 0;

    for case in test_cases.into_iter() {
        let name = case
            .path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown".into());

        let outcome = match case
            .run(&source, definition.clone(), &mut get_context())
            .await
        {
            Ok(outcome) => outcome,
            Err(e) => Outcome::Run { err: e.to_string() },
        };

        println!("{name} {} ", outcome.summary());

        if !outcome.is_success() {
            failures.push((name, outcome));
        } else {
            passed += 1;
        }
    }

    if !failures.is_empty() {
        println!("\n━━━ FAILURES ━━━\n");
        for (name, outcome) in failures.iter() {
            println!("── {name} ──");
            if let Some(detail) = outcome.detail() {
                println!("{detail}");
            }
        }
    }

    let failed = failures.len();
    println!("\n{passed}/{total} passed, {failed} failed");

    if failed > 0 {
        Err(anyhow!("{failed} test(s) failed"))
    } else {
        Ok(())
    }
}

#[derive(Debug)]
struct TestCase {
    path: PathBuf,
    // expect_failure: bool,
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

            // let expect_failure = path.join(EXPECTED_RUN_ERROR_FILE).exists();
            let expected_output_dir = path.join(EXPECTED_RUN_OUTPUT_DIR);
            // if !expect_failure {
            if !expected_output_dir.exists() {
                return Err(io_err(io::ErrorKind::NotFound, &vars_file));
            } else if !expected_output_dir.is_dir() {
                return Err(io_err(io::ErrorKind::NotADirectory, &expected_output_dir));
            }
            // }

            test_cases.push(TestCase {
                path,
                // expect_failure,
            });
        }

        test_cases.sort_by(|l, r| l.path.cmp(&r.path));

        Ok(test_cases)
    }

    async fn run(
        self,
        source: &Source,
        mut definition: CustomProviderDefinition,
        ctx: &mut impl ResolutionContext,
    ) -> anyhow::Result<Outcome> {
        debug!("building templating context");
        let template_ctx = TemplateContext::new(
            load_variables(&self.path.join(VARIABLES_FILE))?,
            source.clone(),
            Default::default(),
            Default::default(),
        );

        debug!("templating provider");
        if let Err(e) = definition.try_template(&mut Vec::new(), source, &template_ctx) {
            return Ok(Outcome::Template { err: e.to_string() });
        }
        debug!("running checks");
        if let Err(e) = definition.command.try_check(&mut Vec::new(), ctx) {
            return Ok(Outcome::Check { err: e.to_string() });
        }

        // We run the provider in an self-removing temp directory so we don't need to worry about
        // manual cleanup of test data.
        debug!("creating temp directory for test output");
        let tmp_dir = TempDir::new()?;
        let out_dir = tmp_dir.path();
        let output_dir = out_dir.join(OUTPUT_PATH);
        let provider_dir = out_dir.join(PROVIDER_DIR);

        debug!("running provider");
        let res = definition
            .command
            .run_providers_and_execute(out_dir, output_dir.clone(), provider_dir, ctx)
            .await;

        if let Err(e) = res {
            return Ok(Outcome::Run { err: e.to_string() });
        }

        // check that the output is as expected
        debug!("processing output");
        let actual = output_file_results(&output_dir)?;
        let expected = output_file_results(&self.path.join(EXPECTED_RUN_OUTPUT_DIR))?;

        let mut missing = Vec::new();
        let mut unexpected = Vec::new();
        let mut with_diff = Vec::new();

        for (file_path, _) in expected.iter() {
            if !actual.contains_key(file_path) {
                missing.push(file_path.to_owned());
                continue;
            }
        }

        for (file_path, actual_file) in actual.iter() {
            let expected_file = match expected.get(file_path) {
                Some(s) => s,
                None => {
                    unexpected.push(file_path.to_owned());
                    continue;
                }
            };
            let diff = TextDiff::from_lines(expected_file, actual_file);
            let mut s = String::new();

            for change in diff.iter_all_changes() {
                let sign = match change.tag() {
                    ChangeTag::Delete => "-",
                    ChangeTag::Insert => "+",
                    ChangeTag::Equal => continue,
                };
                s.push_str(&format!("{sign}{change}"));
            }

            if !s.is_empty() {
                with_diff.push((file_path.to_owned(), s));
            }
        }

        if missing.is_empty() && unexpected.is_empty() && with_diff.is_empty() {
            Ok(Outcome::Success)
        } else {
            Ok(Outcome::OutputDiff {
                missing,
                unexpected,
                with_diff,
            })
        }
    }
}

/// Outcome from running the test case that relate to the structure or behaviour of the
/// custom provider itself.
///
/// Failures from IO around setting up and running the test itself are reported separately
enum Outcome {
    Success,

    Template {
        err: String,
    },

    Check {
        err: String,
    },

    Run {
        err: String,
    },

    OutputDiff {
        missing: Vec<String>,
        unexpected: Vec<String>,
        with_diff: Vec<(String, String)>,
    },
    // ExpectedError {
    //     expected: String,
    //     actual: String,
    // },
}

impl Outcome {
    fn is_success(&self) -> bool {
        matches!(self, Outcome::Success)
    }

    fn summary(&self) -> &'static str {
        match self {
            Outcome::Success => "passed",
            Outcome::Template { .. } => "template error",
            Outcome::Check { .. } => "check failed",
            Outcome::Run { .. } => "run error",
            Outcome::OutputDiff { .. } => "output mismatch",
        }
    }

    fn detail(&self) -> Option<String> {
        match self {
            Outcome::Success => None,
            Outcome::Template { err } => Some(format!("Template error:\n  {err}")),
            Outcome::Check { err } => Some(format!("Check failed:\n  {err}")),
            Outcome::Run { err } => Some(format!("Run error:\n  {err}")),
            Outcome::OutputDiff {
                missing,
                unexpected,
                with_diff,
            } => {
                let mut s = String::new();
                for f in missing {
                    s.push_str(&format!("  missing: {f}\n"));
                }
                for f in unexpected {
                    s.push_str(&format!("  unexpected: {f}\n"));
                }
                for (path, diff) in with_diff {
                    s.push_str(&format!("  {path}:\n{diff}\n"));
                }

                Some(s)
            }
        }
    }
}

fn load_variables(path: &Path) -> anyhow::Result<HashMap<String, Scalar>> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("Failed to read variables file: {}", path.display()))?;
    let vars: HashMap<String, Scalar> = serde_json::from_str(&content)
        .with_context(|| format!("Failed to parse variables JSON: {}", path.display()))?;

    Ok(vars)
}

fn output_file_results(p: &Path) -> io::Result<HashMap<String, String>> {
    WalkDir::new(p)
        .into_iter()
        .filter_entry(|entry| entry.path().is_file())
        .map(|entry| {
            entry.map_err(Into::into).and_then(|e| {
                fs::read_to_string(e.path())
                    .map(|s| (e.path().strip_prefix(p).unwrap().display().to_string(), s))
            })
        })
        .collect()
}
