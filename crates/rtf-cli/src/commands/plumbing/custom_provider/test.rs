//! Run validation tests for a custom provider
use crate::commands::{get_context, plumbing::custom_provider::load_definition};
use anyhow::{Context as _, anyhow};
use assert_fs::TempDir;
use rtf_config::{
    SourceDir,
    checks::Check,
    context::{Context, ResolutionContext},
    formats::CustomProviderDefinition,
    providers::command::{OUTPUT_PATH, PROVIDER_DIR},
    templating::{Scalar, Template, TemplateContext},
};
use similar::{ChangeTag, TextDiff};
use std::{
    collections::HashMap,
    fs,
    io::{self, IsTerminal, stdout},
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, Instant},
};
use tracing::{debug, error, warn};
use walkdir::WalkDir;

const ANSI_GREEN: &str = "\x1b[32m";
const ANSI_RED: &str = "\x1b[31m";
const ANSI_RESET: &str = "\x1b[0m";
const EXPECTED_COPIED_FILES: &str = "expected-copied-files.json";
const EXPECTED_RUN_ERROR_FILE: &str = "expected-run-error.txt";
const EXPECTED_RUN_OUTPUT_DIR: &str = "expected-run-output";
const EXPECTED_RUN_OUTPUT_FILE: &str = "expected-run-output.txt";
const RTF_OUTPUT: &str = "RTF_OUTPUT";
const TEST_CASES_DIR: &str = "test-cases";
const VARIABLES_FILE: &str = "variables.json";

pub async fn test_custom_provider(
    definition_path: &str,
    test_cases_dir: Option<String>,
    error_on_empty: bool,
    no_capture: bool,
) -> anyhow::Result<()> {
    warn!("This is an experimental sub-command that is subject to changes in behaviour!");

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
    let total_start = Instant::now();
    let is_tty = stdout().is_terminal();

    let mut outcomes: Vec<Outcome> = Vec::with_capacity(total);
    let rx = spawn_cases(definition, source, test_cases);

    for result in rx.into_iter() {
        println!(
            "{}",
            format_result_line(&result.name, &result.outcome, result.elapsed, is_tty)
        );

        if !result.outcome.is_success() {
            let detail = result.outcome.detail();
            if !detail.is_empty() {
                println!("{}", format_diff_detail(&detail, is_tty));
            }

            if no_capture {
                print_captured_output(&result.captured_stdout, &result.captured_stderr, is_tty);
            }
        }

        outcomes.push(result.outcome);
    }

    let total_elapsed = total_start.elapsed();
    let passed = outcomes.iter().filter(|o| o.is_success()).count();
    let failures: Vec<_> = outcomes.iter().filter(|o| !o.is_success()).collect();
    let failed = failures.len();

    let (color_start, color_end) = if is_tty {
        (if failed > 0 { ANSI_RED } else { ANSI_GREEN }, ANSI_RESET)
    } else {
        ("", "")
    };

    println!(
        "\n {color_start}Summary{color_end} [{:>7.3}s] {} tests run: {} passed, {} failed",
        total_elapsed.as_secs_f64(),
        total,
        passed,
        failed
    );

    if failed > 0 {
        Err(anyhow!("{failed} test(s) failed"))
    } else {
        Ok(())
    }
}

fn spawn_cases(
    definition: CustomProviderDefinition,
    source: SourceDir,
    test_cases: Vec<TestCase>,
) -> Receiver<TestResult> {
    let (tx, rx) = mpsc::channel();

    for case in test_cases.into_iter() {
        let source = source.clone();
        let definition = definition.clone();
        let tx = tx.clone();

        thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("failed to create tokio runtime");

            rt.block_on(async {
                let name = case
                    .path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "unknown".into());

                // We need a fresh context for each test to avoid incorrectly trying to use cached
                // provider output that was just removed in the temp dir of a previous run
                let mut ctx = get_context();
                ctx.enable_output_capture();

                let start = Instant::now();
                let outcome = match case.run(&source, definition, &mut ctx).await {
                    Ok(outcome) => outcome,
                    Err(e) => Outcome::Run { err: e.to_string() },
                };

                let _ = tx.send(TestResult {
                    name,
                    outcome,
                    elapsed: start.elapsed(),
                    captured_stdout: ctx.captured_stdout(),
                    captured_stderr: ctx.captured_stderr(),
                });
            });
        });
    }

    // Dropping the sender when we return ensures that the channel closes when all threads finish

    rx
}

#[derive(Debug)]
struct TestResult {
    name: String,
    outcome: Outcome,
    elapsed: Duration,
    captured_stdout: String,
    captured_stderr: String,
}

fn io_err(kind: io::ErrorKind, p: &Path) -> io::Error {
    io::Error::new(kind, p.display().to_string())
}

fn expect_file(p: &Path) -> io::Result<()> {
    if !p.exists() {
        return Err(io_err(io::ErrorKind::NotFound, p));
    } else if p.is_dir() {
        return Err(io_err(io::ErrorKind::IsADirectory, p));
    }

    Ok(())
}

fn expect_dir(p: &Path) -> io::Result<()> {
    if !p.exists() {
        return Err(io_err(io::ErrorKind::NotFound, p));
    } else if !p.is_dir() {
        return Err(io_err(io::ErrorKind::NotADirectory, p));
    }

    Ok(())
}

#[derive(Debug)]
enum TestCaseData {
    Failure(String),
    SuccessFile(String),
    SuccessDir(HashMap<String, String>),
}

#[derive(Debug)]
struct TestCase {
    path: PathBuf,
    data: TestCaseData,
}

impl TestCase {
    fn is_expected_failure(&self) -> bool {
        matches!(self.data, TestCaseData::Failure(_))
    }

    #[cfg(test)]
    fn is_expected_success(&self) -> bool {
        matches!(
            self.data,
            TestCaseData::SuccessDir(_) | TestCaseData::SuccessFile(_)
        )
    }

    fn try_load_all(dir: &Path) -> anyhow::Result<Vec<Self>> {
        expect_dir(dir)?;

        let mut test_cases = Vec::new();

        for entry in dir.read_dir()? {
            let entry = entry?;
            let path = entry.path();
            expect_dir(&path)?;

            let vars_file = path.join(VARIABLES_FILE);
            expect_file(&vars_file)?;

            // Ensure that we have exactly one test type present for the case
            let expected_output_dir = path.join(EXPECTED_RUN_OUTPUT_DIR);
            let expected_output_file = path.join(EXPECTED_RUN_OUTPUT_FILE);
            let expected_failure_file = path.join(EXPECTED_RUN_ERROR_FILE);

            let types_present = [
                &expected_output_dir,
                &expected_output_file,
                &expected_failure_file,
            ]
            .into_iter()
            .filter(|p| p.exists())
            .count();

            let invalid_assertion_data_msg = || {
                format!(
                    "Expected one of:\n- {EXPECTED_RUN_OUTPUT_FILE}\n- {EXPECTED_RUN_OUTPUT_DIR}\n- {EXPECTED_RUN_ERROR_FILE}"
                )
            };

            let mut expected_files = load_expected_copied_files(&path)?;

            if types_present == 0 && expected_files.is_empty() {
                return Err(anyhow!(
                    "No test assertion data found for {}\n{}",
                    path.display(),
                    invalid_assertion_data_msg()
                ));
            } else if types_present > 1 {
                return Err(anyhow!(
                    "Conflicting test assertion data found for {}\n{}",
                    path.display(),
                    invalid_assertion_data_msg()
                ));
            }

            let data = if expected_failure_file.exists() {
                expect_file(&expected_failure_file)?;
                if !expected_files.is_empty() {
                    return Err(anyhow!(
                        "Invalid test case '{}': {EXPECTED_COPIED_FILES} can not be used with {EXPECTED_RUN_ERROR_FILE}",
                        path.display()
                    ));
                }

                TestCaseData::Failure(fs::read_to_string(&expected_failure_file).with_context(
                    || format!("unable to read {}", expected_failure_file.display()),
                )?)
            } else if expected_output_dir.exists() {
                expect_dir(&expected_output_dir)?;
                let explicit_files = output_files(&expected_output_dir)?;
                let conflicts: Vec<_> = expected_files
                    .keys()
                    .filter(|k| explicit_files.contains_key(*k))
                    .cloned()
                    .collect();

                if !conflicts.is_empty() {
                    return Err(anyhow!(
                        "Invalid test case '{}': {EXPECTED_COPIED_FILES} and {EXPECTED_RUN_OUTPUT_DIR} have conflicting paths - {}",
                        path.display(),
                        conflicts.join(", ")
                    ));
                }

                expected_files.extend(explicit_files);

                TestCaseData::SuccessDir(expected_files)
            } else if expected_output_file.exists() {
                expect_file(&expected_output_file)?;
                if !expected_files.is_empty() {
                    return Err(anyhow!(
                        "Invalid test case '{}': {EXPECTED_COPIED_FILES} can not be used with {EXPECTED_RUN_OUTPUT_FILE}",
                        path.display()
                    ));
                }

                TestCaseData::SuccessFile(fs::read_to_string(&expected_output_file).with_context(
                    || format!("unable to read {}", expected_output_file.display()),
                )?)
            } else {
                match expected_files.remove(RTF_OUTPUT) {
                    Some(s) if expected_files.is_empty() => TestCaseData::SuccessFile(s),
                    Some(_) => {
                        return Err(anyhow!(
                            "{RTF_OUTPUT} must be the only copied file key when present"
                        ));
                    }
                    None if expected_files.is_empty() => {
                        return Err(anyhow!("{}", invalid_assertion_data_msg()));
                    }
                    None => TestCaseData::SuccessDir(expected_files),
                }
            };

            test_cases.push(TestCase { path, data });
        }

        test_cases.sort_by(|l, r| l.path.cmp(&r.path));

        Ok(test_cases)
    }

    fn check_expected_failure(&self, stdout: String, stderr: String) -> Outcome {
        let expected = match &self.data {
            TestCaseData::Failure(s) => s,
            _ => {
                panic!("attempt to check expected failure for a test that expected to pass")
            }
        };

        let mut actual = String::new();
        if !stdout.is_empty() {
            actual = format!("stdout: {stdout}");
        }
        if !stderr.is_empty() {
            let join = if actual.is_empty() { "" } else { "\n" };
            actual = format!("{actual}{join}stderr: {stderr}");
        }

        if &actual == expected {
            Outcome::Success
        } else {
            Outcome::ExpectedError {
                expected: expected.clone(),
                actual,
            }
        }
    }

    async fn run(
        self,
        source: &SourceDir,
        mut definition: CustomProviderDefinition,
        ctx: &mut Context,
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
            return if self.is_expected_failure() {
                let stdout = ctx.captured_stdout();
                let stderr = ctx.captured_stderr();
                Ok(self.check_expected_failure(stdout, stderr))
            } else {
                Ok(Outcome::Template { err: e.to_string() })
            };
        }
        debug!("running checks");
        if let Err(e) = definition.command.try_check(&mut Vec::new(), ctx) {
            return if self.is_expected_failure() {
                let stdout = ctx.captured_stdout();
                let stderr = ctx.captured_stderr();
                Ok(self.check_expected_failure(stdout, stderr))
            } else {
                Ok(Outcome::Check { err: e.to_string() })
            };
        }

        // We run the provider in an self-removing temp directory so we don't need to worry about
        // manual cleanup of test data.
        debug!("creating temp directory for test output");
        let tmp_dir = TempDir::new()?;
        let out_dir = tmp_dir.path();
        let output_path = out_dir.join(OUTPUT_PATH);
        let provider_dir = out_dir.join(PROVIDER_DIR);

        debug!("running provider");
        let res = definition
            .command
            .run_providers_and_execute("test", out_dir, output_path.clone(), provider_dir, ctx)
            .await;

        if let Err(e) = res {
            return if self.is_expected_failure() {
                let stdout = ctx.captured_stdout();
                let stderr = ctx.captured_stderr();
                Ok(self.check_expected_failure(stdout, stderr))
            } else {
                Ok(Outcome::Run { err: e.to_string() })
            };
        }

        debug!("processing output");
        let actual: HashMap<String, String> = if output_path.is_file() {
            let content = fs::read_to_string(&output_path)?;
            HashMap::from([(RTF_OUTPUT.to_string(), normalize_paths(&content, out_dir))])
        } else {
            output_files(&output_path)?
                .into_iter()
                .map(|(k, v)| (k, normalize_paths(&v, out_dir)))
                .collect()
        };

        let expected = match &self.data {
            TestCaseData::SuccessDir(m) => m,

            TestCaseData::SuccessFile(s) => &HashMap::from([(
                RTF_OUTPUT.to_string(),
                normalize_paths(&s.to_owned(), out_dir),
            )]),

            TestCaseData::Failure(s) => {
                error!("Expected-failure case passed. Expected: {s}");
                &HashMap::new()
            }
        };

        Ok(compare_outputs(&actual, expected))
    }
}

/// Outcome from running the test case that relate to the structure or behaviour of the
/// custom provider itself.
///
/// Failures from IO around setting up and running the test itself are reported separately
#[derive(Debug)]
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

    ExpectedError {
        expected: String,
        actual: String,
    },
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
            Outcome::ExpectedError { .. } => "wrong error output",
        }
    }

    fn detail(&self) -> String {
        match self {
            Outcome::Success => String::new(),
            Outcome::Template { err } => format!("Template error:\n  {err}"),
            Outcome::Check { err } => format!("Check failed:\n  {err}"),
            Outcome::Run { err } => format!("Run error:\n  {err}"),
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

                s
            }

            Outcome::ExpectedError { expected, actual } => {
                format!("expected:\n{expected}\n\n  actual:\n{actual}")
            }
        }
    }
}

fn compare_outputs(
    actual: &HashMap<String, String>,
    expected: &HashMap<String, String>,
) -> Outcome {
    let mut missing = Vec::new();
    let mut unexpected = Vec::new();
    let mut with_diff = Vec::new();

    for file_path in expected.keys() {
        if !actual.contains_key(file_path) {
            missing.push(file_path.to_owned());
        }
    }

    for (file_path, actual_content) in actual.iter() {
        let expected_content = match expected.get(file_path) {
            Some(s) => s,
            None => {
                unexpected.push(file_path.to_owned());
                continue;
            }
        };
        let diff = TextDiff::from_lines(expected_content, actual_content);
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
        Outcome::Success
    } else {
        Outcome::OutputDiff {
            missing,
            unexpected,
            with_diff,
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

fn load_expected_copied_files(test_case_path: &Path) -> anyhow::Result<HashMap<String, String>> {
    let index_path = test_case_path.join(EXPECTED_COPIED_FILES);
    if !index_path.exists() {
        return Ok(HashMap::new());
    }

    let content = fs::read_to_string(&index_path).with_context(|| {
        format!(
            "Failed to read copied files index: {}",
            index_path.display()
        )
    })?;

    let raw_index: HashMap<String, String> = serde_json::from_str(&content).with_context(|| {
        format!(
            "Failed to parse copied files index: {}",
            index_path.display()
        )
    })?;

    let mut expected = HashMap::new();
    for (output_path, relative_source) in raw_index {
        let source_path = test_case_path.join(&relative_source);
        let source_path = source_path.canonicalize().with_context(|| {
            format!(
                "Source file not found for '{}': {}",
                output_path,
                source_path.display()
            )
        })?;
        let content = fs::read_to_string(source_path)?;
        expected.insert(output_path, content);
    }

    Ok(expected)
}

/// Replace occurrences of the temp directory path with `$OUTDIR` placeholder.
///
/// This allows expected output files to use `$OUTDIR` as a placeholder that matches
/// against the actual absolute temp directory path used during test execution.
#[inline]
fn normalize_paths(content: &str, temp_dir: &Path) -> String {
    content.replace(&temp_dir.display().to_string(), "$OUTDIR")
}

fn output_files(p: &Path) -> io::Result<HashMap<String, String>> {
    let mut files = HashMap::new();

    for entry in WalkDir::new(p) {
        let entry = entry.map_err(|e| io::Error::other(e.to_string()))?;
        if entry.path().is_file() {
            let content = fs::read_to_string(entry.path())?;
            let key = entry.path().strip_prefix(p).unwrap().display().to_string();
            files.insert(key, content);
        }
    }

    Ok(files)
}

fn format_result_line(name: &str, outcome: &Outcome, elapsed: Duration, is_tty: bool) -> String {
    let status = if outcome.is_success() { "PASS" } else { "FAIL" };
    let mut color_start = "";
    let mut color_end = "";

    if is_tty {
        if outcome.is_success() {
            color_start = ANSI_GREEN;
        } else {
            color_start = ANSI_RED;
        };

        color_end = ANSI_RESET;
    }
    let secs = elapsed.as_secs_f64();

    if outcome.is_success() {
        format!("{color_start}{status:>8}{color_end} [{secs:>7.3}s] {name}")
    } else {
        format!(
            "{color_start}{status:>8}{color_end} [{secs:>7.3}s] {name} - {}",
            outcome.summary()
        )
    }
}

fn diff_line(line: &str, is_tty: bool) -> String {
    if !is_tty {
        return line.to_string();
    }

    if let Some(rest) = line.strip_prefix('-') {
        format!("{ANSI_RED}-{rest}{ANSI_RESET}")
    } else if let Some(rest) = line.strip_prefix('+') {
        format!("{ANSI_GREEN}+{rest}{ANSI_RESET}")
    } else {
        line.to_string()
    }
}

fn format_diff_detail(detail: &str, is_tty: bool) -> String {
    detail
        .lines()
        .map(|line| format!("        {}", diff_line(line, is_tty)))
        .collect::<Vec<_>>()
        .join("\n")
}

fn print_captured_output(stdout: &str, stderr: &str, is_tty: bool) {
    let (color_start, color_end) = if is_tty {
        (ANSI_RED, ANSI_RESET)
    } else {
        ("", "")
    };

    if !stdout.is_empty() {
        println!("\n        {color_start}--- stdout ---{color_end}");
        for line in stdout.lines() {
            println!("        {line}");
        }
    }
    if !stderr.is_empty() {
        println!("\n        {color_start}--- stderr ---{color_end}");
        for line in stderr.lines() {
            println!("        {line}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_fs::{TempDir, prelude::*};
    use simple_test_case::test_case;

    #[test]
    fn output_file_results_collects_all_files() {
        let tmp = TempDir::new().unwrap();
        tmp.child("root.txt").write_str("root content\n").unwrap();
        tmp.child("nested/child.txt")
            .write_str("nested content\n")
            .unwrap();

        let res = output_files(tmp.path()).expect("should read directory");

        assert_eq!(res.len(), 2, "should find 2 files");
        assert_eq!(
            res.get("root.txt").map(|s| s.as_str()),
            Some("root content\n"),
            "should find root.txt with correct content"
        );
        assert_eq!(
            res.get("nested/child.txt").map(|s| s.as_str()),
            Some("nested content\n"),
            "should find nested/child.txt with correct content"
        );
    }

    #[test]
    fn output_file_results_empty_directory() {
        let tmp = TempDir::new().unwrap();
        let res = output_files(tmp.path()).expect("should read directory");
        assert_eq!(res.len(), 0, "empty directory should return empty HashMap");
    }

    #[test]
    fn output_file_results_nested_files_only() {
        let tmp = TempDir::new().unwrap();
        tmp.child("subdir/file.txt")
            .write_str("nested content\n")
            .unwrap();

        let res = output_files(tmp.path()).expect("should read directory");

        assert_eq!(res.len(), 1, "should find 1 nested file");
        assert_eq!(
            res.get("subdir/file.txt").map(|s| s.as_str()),
            Some("nested content\n"),
            "should find nested file with correct content"
        );
    }

    #[test]
    fn output_file_results_nonexistent_directory() {
        let tmp = TempDir::new().unwrap();
        let nonexistent = tmp.path().join("does-not-exist");

        let res = output_files(&nonexistent);

        assert!(res.is_err(), "non-existent directory should return error");
    }

    #[test_case(Outcome::Success, true; "success")]
    #[test_case(Outcome::Template { err: "e".into() }, false; "template")]
    #[test_case(Outcome::Check { err: "e".into() }, false; "check")]
    #[test_case(Outcome::Run { err: "e".into() }, false; "run")]
    #[test_case(Outcome::OutputDiff { missing: vec![], unexpected: vec![], with_diff: vec![] }, false; "output diff")]
    #[test_case(Outcome::ExpectedError { expected: "e".into(), actual: "a".into() }, false; "expected error")]
    #[test]
    fn outcome_is_success(outcome: Outcome, expected: bool) {
        assert_eq!(outcome.is_success(), expected);
    }

    #[test_case(Outcome::Success, "passed"; "success")]
    #[test_case(Outcome::Template { err: "e".into() }, "template error"; "template")]
    #[test_case(Outcome::Check { err: "e".into() }, "check failed"; "check")]
    #[test_case(Outcome::Run { err: "e".into() }, "run error"; "run")]
    #[test_case(Outcome::OutputDiff { missing: vec![], unexpected: vec![], with_diff: vec![] }, "output mismatch"; "output diff")]
    #[test_case(Outcome::ExpectedError { expected: "e".into(), actual: "a".into() }, "wrong error output"; "expected error")]
    #[test]
    fn outcome_summary(outcome: Outcome, expected: &str) {
        assert_eq!(outcome.summary(), expected);
    }

    #[test_case(Outcome::Success, ""; "success returns empty")]
    #[test_case(
        Outcome::Template { err: "variable not found".into() },
        "Template error:\n  variable not found";
        "template formats error"
    )]
    #[test_case(
        Outcome::Check { err: "file not found".into() },
        "Check failed:\n  file not found";
        "check formats error"
    )]
    #[test_case(
        Outcome::Run { err: "command failed".into() },
        "Run error:\n  command failed";
        "run formats error"
    )]
    #[test_case(
        Outcome::OutputDiff {
            missing: vec![],
            unexpected: vec![],
            with_diff: vec![]
        },
        "";
        "output diff empty"
    )]
    #[test_case(
        Outcome::OutputDiff {
            missing: vec!["a.txt".into(), "b.txt".into()],
            unexpected: vec![],
            with_diff: vec![]
        },
        "  missing: a.txt\n  missing: b.txt\n";
        "output diff missing files"
    )]
    #[test_case(
        Outcome::OutputDiff {
            missing: vec![],
            unexpected: vec!["extra.log".into()],
            with_diff: vec![]
        },
        "  unexpected: extra.log\n";
        "output diff unexpected files"
    )]
    #[test_case(
        Outcome::OutputDiff {
            missing: vec![],
            unexpected: vec![],
            with_diff: vec![("config.json".into(), "-old\n+new\n".into())]
        },
        "  config.json:\n-old\n+new\n\n";
        "output diff with diffs"
    )]
    #[test_case(
        Outcome::OutputDiff {
            missing: vec!["m.txt".into()],
            unexpected: vec!["u.txt".into()],
            with_diff: vec![("d.txt".into(), "-a\n+b\n".into())]
        },
        "  missing: m.txt\n  unexpected: u.txt\n  d.txt:\n-a\n+b\n\n";
        "output diff combined"
    )]
    #[test_case(
        Outcome::ExpectedError { expected: "expected msg".into(), actual: "actual msg".into() },
        "expected:\nexpected msg\n\n  actual:\nactual msg";
        "expected error formats both"
    )]
    #[test]
    fn outcome_detail(outcome: Outcome, expected: &str) {
        assert_eq!(outcome.detail(), expected);
    }

    #[test_case(r#"{"key": "value", "num": 42, "flag": true}"#, 3; "valid with entries")]
    #[test_case(r#"{}"#, 0; "empty object")]
    #[test]
    fn load_variables_valid(json_content: &str, expected_count: usize) {
        let tmp = TempDir::new().unwrap();
        tmp.child("variables.json").write_str(json_content).unwrap();

        let res = load_variables(&tmp.path().join("variables.json"));

        assert!(res.is_ok());
        assert_eq!(res.unwrap().len(), expected_count);
    }

    #[test_case("nonexistent.json", None; "missing file")]
    #[test_case("malformed.json", Some("{not valid json"); "malformed json")]
    #[test_case("variables.json", Some(r#"{"key": [1, 2, 3]}"#); "array value")]
    #[test_case("variables.json", Some(r#"{"key": {"nested": "value"}}"#); "nested object")]
    #[test]
    fn load_variables_returns_error(filename: &str, content: Option<&str>) {
        let tmp = TempDir::new().unwrap();
        if let Some(content) = content {
            tmp.child(filename).write_str(content).unwrap();
        }

        let res = load_variables(&tmp.path().join(filename));

        assert!(res.is_err());
    }

    #[test]
    fn load_expected_copied_files_nonexistent_returns_empty() {
        let tmp = TempDir::new().unwrap();
        let expected = load_expected_copied_files(tmp.path()).expect("should succeed");

        assert_eq!(expected, HashMap::new());
    }

    #[test]
    fn load_expected_copied_files_empty_object_valid() {
        let tmp = TempDir::new().unwrap();
        tmp.child(EXPECTED_COPIED_FILES).write_str("{}").unwrap();
        let expected = load_expected_copied_files(tmp.path()).expect("should succeed");

        assert_eq!(expected, HashMap::new());
    }

    #[test]
    fn load_expected_copied_files_resolves_paths() {
        let tmp = TempDir::new().unwrap();
        tmp.child("source/file.txt")
            .write_str("source content")
            .unwrap();
        tmp.child("case").create_dir_all().unwrap();
        tmp.child(format!("case/{EXPECTED_COPIED_FILES}"))
            .write_str(r#"{"output.txt": "../source/file.txt"}"#)
            .unwrap();

        let res = load_expected_copied_files(&tmp.path().join("case")).expect("should succeed");

        assert_eq!(res.len(), 1, "should have one entry");
        let content = res.get("output.txt").expect("should have output.txt key");
        assert_eq!(content, "source content");
    }

    #[test]
    fn load_expected_copied_files_missing_source_errors() {
        let tmp = TempDir::new().unwrap();
        tmp.child(EXPECTED_COPIED_FILES)
            .write_str(r#"{"output.txt": "nonexistent.txt"}"#)
            .unwrap();

        let res = load_expected_copied_files(tmp.path());

        assert!(res.is_err(), "should error when source file doesn't exist");
        let err = res.unwrap_err().to_string();
        assert!(err.contains("Source file not found"), "{err}");
        assert!(err.contains("output.txt"), "{err}");
    }

    #[test]
    fn load_expected_copied_files_malformed_json_errors() {
        let tmp = TempDir::new().unwrap();
        tmp.child(EXPECTED_COPIED_FILES)
            .write_str("{not valid json")
            .unwrap();

        let res = load_expected_copied_files(tmp.path());

        assert!(res.is_err(), "should error on malformed JSON");
        let err = res.unwrap_err().to_string();
        assert!(err.contains("Failed to parse copied files index"), "{err}");
    }

    fn create_valid_test_case(tmp: &TempDir, name: &str) {
        tmp.child(format!("{name}/variables.json"))
            .write_str("{}")
            .unwrap();
        tmp.child(format!("{name}/expected-run-output"))
            .create_dir_all()
            .unwrap();
    }

    fn create_failure_test_case(tmp: &TempDir, name: &str, expected_error: &str) {
        tmp.child(format!("{name}/variables.json"))
            .write_str("{}")
            .unwrap();
        tmp.child(format!("{name}/expected-run-error.txt"))
            .write_str(expected_error)
            .unwrap();
    }

    #[test_case(0; "empty directory")]
    #[test_case(1; "single test case")]
    #[test_case(3; "multiple test cases")]
    #[test]
    fn try_load_all_valid(case_count: usize) {
        let tmp = TempDir::new().unwrap();
        for i in 0..case_count {
            create_valid_test_case(&tmp, &format!("case-{i}"));
        }

        let res = TestCase::try_load_all(tmp.path());

        assert!(res.is_ok(), "try_load_all should succeed: {res:?}");
        assert_eq!(res.unwrap().len(), case_count);
    }

    #[test]
    fn try_load_all_returns_sorted_test_cases() {
        let tmp = TempDir::new().unwrap();
        create_valid_test_case(&tmp, "zebra");
        create_valid_test_case(&tmp, "alpha");
        create_valid_test_case(&tmp, "middle");

        let res = TestCase::try_load_all(tmp.path()).unwrap();

        let names: Vec<_> = res
            .iter()
            .map(|tc| tc.path.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(names, vec!["alpha", "middle", "zebra"]);
    }

    #[test]
    fn try_load_all_failure_case_sets_expect_failure() {
        let tmp = TempDir::new().unwrap();
        create_failure_test_case(&tmp, "fail-case", "expected error");

        let cases = TestCase::try_load_all(tmp.path()).unwrap();

        assert_eq!(cases.len(), 1);
        assert!(cases[0].is_expected_failure());
    }

    #[test]
    fn try_load_all_mixed_success_and_failure() {
        let tmp = TempDir::new().unwrap();
        create_valid_test_case(&tmp, "success-case");
        create_failure_test_case(&tmp, "fail-case", "error");

        let cases = TestCase::try_load_all(tmp.path()).unwrap();

        assert_eq!(cases.len(), 2);
        // Sorted alphabetically: fail-case, success-case
        assert!(cases[0].is_expected_failure());
        assert!(cases[1].is_expected_success());
    }

    #[test]
    fn try_load_all_with_copied_files_only() {
        let tmp = TempDir::new().unwrap();
        tmp.child("case/variables.json").write_str("{}").unwrap();
        tmp.child("case/source.txt")
            .write_str("source content")
            .unwrap();
        tmp.child(format!("case/{EXPECTED_COPIED_FILES}"))
            .write_str(r#"{"output.txt": "source.txt"}"#)
            .unwrap();

        let cases = TestCase::try_load_all(tmp.path()).expect("should succeed");

        assert_eq!(cases.len(), 1);

        let expected = match &cases[0].data {
            TestCaseData::SuccessDir(m) => m,
            _ => panic!("should have been a success dir case"),
        };

        assert_eq!(expected.len(), 1);
        assert!(expected.contains_key("output.txt"));
    }

    #[test]
    fn try_load_all_without_data_errors() {
        let tmp = TempDir::new().unwrap();
        tmp.child("case/variables.json").write_str("{}").unwrap();

        let res = TestCase::try_load_all(tmp.path());

        assert!(res.is_err(), "expected error, got {res:?}");
    }

    #[test]
    fn try_load_all_loads_copied_files_index() {
        let tmp = TempDir::new().unwrap();
        tmp.child("case/variables.json").write_str("{}").unwrap();
        tmp.child("case/expected-run-output")
            .create_dir_all()
            .unwrap();
        tmp.child("case/sources/a.txt")
            .write_str("content a")
            .unwrap();
        tmp.child("case/sources/b.txt")
            .write_str("content b")
            .unwrap();
        tmp.child(format!("case/{EXPECTED_COPIED_FILES}"))
            .write_str(r#"{"out/a.txt": "sources/a.txt", "out/b.txt": "sources/b.txt"}"#)
            .unwrap();

        let cases = TestCase::try_load_all(tmp.path()).expect("should succeed");

        assert_eq!(cases.len(), 1);

        let expected = match &cases[0].data {
            TestCaseData::SuccessDir(m) => m,
            _ => panic!("should have been a success dir case"),
        };

        assert_eq!(expected.len(), 2);
        assert!(expected.contains_key("out/a.txt"));
        assert!(expected.contains_key("out/b.txt"));
    }

    #[test]
    fn try_load_all_conflict_errors() {
        let tmp = TempDir::new().unwrap();
        // Create test case where same path appears in both index and expected-run-output
        tmp.child("case/variables.json").write_str("{}").unwrap();
        tmp.child("case/source.txt")
            .write_str("source content")
            .unwrap();
        // Index references "conflict.txt"
        tmp.child(format!("case/{EXPECTED_COPIED_FILES}"))
            .write_str(r#"{"conflict.txt": "source.txt"}"#)
            .unwrap();
        // expected-run-output also has "conflict.txt"
        tmp.child("case/expected-run-output/conflict.txt")
            .write_str("explicit content")
            .unwrap();

        let res = TestCase::try_load_all(tmp.path());

        assert!(res.is_err(), "expected error, got {res:?}");
        let err = res.unwrap_err().to_string();
        assert!(err.contains("Invalid test case"), "{err}");
        assert!(err.contains("conflict.txt"), "{err}");
    }

    #[test]
    fn try_load_all_no_conflict_with_different_paths() {
        let tmp = TempDir::new().unwrap();
        // Create test case where index and expected-run-output have different paths
        tmp.child("case/variables.json").write_str("{}").unwrap();
        tmp.child("case/source.txt")
            .write_str("source content")
            .unwrap();

        tmp.child(format!("case/{EXPECTED_COPIED_FILES}"))
            .write_str(r#"{"from-index.txt": "source.txt"}"#)
            .unwrap();

        tmp.child("case/expected-run-output/from-dir.txt")
            .write_str("explicit content")
            .unwrap();

        let cases = TestCase::try_load_all(tmp.path()).expect("should succeed with no conflicts");

        assert_eq!(cases.len(), 1);

        let expected = match &cases[0].data {
            TestCaseData::SuccessDir(m) => m,
            _ => panic!("should have been a success dir case"),
        };

        assert_eq!(expected.len(), 2);
        assert!(expected.contains_key("from-index.txt"));
        assert!(expected.contains_key("from-dir.txt"));
    }

    #[test_case(
        HashMap::from([("a.txt", "content")]),
        HashMap::from([("a.txt", "content")]),
        true;
        "identical single file"
    )]
    #[test_case(
        HashMap::from([("a.txt", "hello"), ("b.txt", "world")]),
        HashMap::from([("a.txt", "hello"), ("b.txt", "world")]),
        true;
        "identical multiple files"
    )]
    #[test_case(
        HashMap::from([]),
        HashMap::from([]),
        true;
        "both empty"
    )]
    #[test_case(
        HashMap::from([("a.txt", "content")]),
        HashMap::from([("a.txt", "different")]),
        false;
        "content differs"
    )]
    #[test_case(
        HashMap::from([]),
        HashMap::from([("a.txt", "content")]),
        false;
        "missing file"
    )]
    #[test_case(
        HashMap::from([("a.txt", "content"), ("b.txt", "extra")]),
        HashMap::from([("a.txt", "content")]),
        false;
        "unexpected file"
    )]
    #[test]
    fn compare_outputs_is_success(
        actual: HashMap<&str, &str>,
        expected: HashMap<&str, &str>,
        should_succeed: bool,
    ) {
        let actual: HashMap<String, String> = actual
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let expected: HashMap<String, String> = expected
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();

        let outcome = compare_outputs(&actual, &expected);
        assert_eq!(outcome.is_success(), should_succeed);
    }

    #[test]
    fn compare_outputs_missing_files() {
        let actual = HashMap::new();
        let expected = HashMap::from([
            ("a.txt".to_string(), "content".to_string()),
            ("b.txt".to_string(), "more".to_string()),
        ]);

        let outcome = compare_outputs(&actual, &expected);

        match outcome {
            Outcome::OutputDiff {
                mut missing,
                unexpected,
                with_diff,
            } => {
                missing.sort();
                assert_eq!(missing, vec!["a.txt", "b.txt"]);
                assert!(unexpected.is_empty());
                assert!(with_diff.is_empty());
            }
            _ => panic!("expected OutputDiff, got {outcome:?}"),
        }
    }

    #[test]
    fn compare_outputs_unexpected_files() {
        let actual = HashMap::from([
            ("extra1.txt".to_string(), "content".to_string()),
            ("extra2.txt".to_string(), "more".to_string()),
        ]);
        let expected = HashMap::new();

        let outcome = compare_outputs(&actual, &expected);

        match outcome {
            Outcome::OutputDiff {
                missing,
                mut unexpected,
                with_diff,
            } => {
                assert!(missing.is_empty());
                unexpected.sort();
                assert_eq!(unexpected, vec!["extra1.txt", "extra2.txt"]);
                assert!(with_diff.is_empty());
            }
            _ => panic!("expected OutputDiff, got {outcome:?}"),
        }
    }

    #[test]
    fn compare_outputs_with_diffs() {
        let actual = HashMap::from([("file.txt".to_string(), "new line\n".to_string())]);
        let expected = HashMap::from([("file.txt".to_string(), "old line\n".to_string())]);

        let outcome = compare_outputs(&actual, &expected);

        match outcome {
            Outcome::OutputDiff {
                missing,
                unexpected,
                with_diff,
            } => {
                assert!(missing.is_empty());
                assert!(unexpected.is_empty());
                assert_eq!(with_diff.len(), 1);
                let (path, diff) = &with_diff[0];
                assert_eq!(path, "file.txt");
                assert!(diff.contains("-old line"));
                assert!(diff.contains("+new line"));
            }
            _ => panic!("expected OutputDiff, got {outcome:?}"),
        }
    }

    #[test]
    fn compare_outputs_combined() {
        let actual = HashMap::from([
            ("changed.txt".to_string(), "new\n".to_string()),
            ("extra.txt".to_string(), "unexpected\n".to_string()),
        ]);
        let expected = HashMap::from([
            ("changed.txt".to_string(), "old\n".to_string()),
            ("missing.txt".to_string(), "gone\n".to_string()),
        ]);

        let outcome = compare_outputs(&actual, &expected);

        match outcome {
            Outcome::OutputDiff {
                missing,
                unexpected,
                with_diff,
            } => {
                assert_eq!(missing, vec!["missing.txt"]);
                assert_eq!(unexpected, vec!["extra.txt"]);
                assert_eq!(with_diff.len(), 1);
                assert_eq!(with_diff[0].0, "changed.txt");
            }
            _ => panic!("expected OutputDiff, got {outcome:?}"),
        }
    }

    enum TestAsset {
        Dir(&'static str),
        File(&'static str, &'static str),
    }

    #[test_case(
        &[],
        Some("does-not-exist"),
        io::ErrorKind::NotFound;
        "nonexistent"
    )]
    #[test_case(
        &[TestAsset::File("a-file.txt", "")],
        Some("a-file.txt"),
        io::ErrorKind::NotADirectory;
        "path is file"
    )]
    #[test_case(
        &[TestAsset::File("not-a-dir.txt", "")],
        None,
        io::ErrorKind::NotADirectory;
        "entry is file"
    )]
    #[test_case(
        &[TestAsset::Dir("case-a/expected-run-output")],
        None,
        io::ErrorKind::NotFound;
        "missing variables"
    )]
    #[test_case(
        &[TestAsset::Dir("case-a/variables.json"), TestAsset::Dir("case-a/expected-run-output")],
        None,
        io::ErrorKind::IsADirectory;
        "variables is dir"
    )]
    #[test_case(
        &[TestAsset::File("case-a/variables.json", "{}"), TestAsset::File("case-a/expected-run-output", "")],
        None,
        io::ErrorKind::NotADirectory;
        "expected output is file"
    )]
    #[test_case(
        &[TestAsset::File("case/variables.json", "{}"), TestAsset::Dir("case/expected-run-error.txt")],
        None,
        io::ErrorKind::IsADirectory;
        "expected error file is dir"
    )]
    #[test]
    fn try_load_all_io_error(assets: &[TestAsset], subpath: Option<&str>, expected: io::ErrorKind) {
        let tmp = TempDir::new().unwrap();
        for asset in assets {
            match asset {
                TestAsset::Dir(p) => tmp.child(*p).create_dir_all().unwrap(),
                TestAsset::File(p, content) => tmp.child(*p).write_str(content).unwrap(),
            }
        }
        let path = match subpath {
            Some(s) => tmp.path().join(s),
            None => tmp.path().to_path_buf(),
        };
        let err = TestCase::try_load_all(&path).unwrap_err();
        assert_eq!(
            err.downcast_ref::<io::Error>().map(|e| e.kind()),
            Some(expected),
            "expected IO error, got {err:?}"
        );
    }

    #[test]
    fn format_result_line_success() {
        let outcome = Outcome::Success;
        let elapsed = Duration::from_millis(42);

        let line = format_result_line("test_name", &outcome, elapsed, false);

        assert_eq!(line, "    PASS [  0.042s] test_name");
    }

    #[test]
    fn format_result_line_failure_includes_reason() {
        let outcome = Outcome::OutputDiff {
            missing: vec![],
            unexpected: vec![],
            with_diff: vec![],
        };
        let elapsed = Duration::from_millis(156);

        let line = format_result_line("test_name", &outcome, elapsed, false);

        assert_eq!(line, "    FAIL [  0.156s] test_name - output mismatch");
    }

    #[test_case(
        Outcome::Template { err: "e".into() },
        "template error";
        "template error"
    )]
    #[test_case(
        Outcome::Check { err: "e".into() },
        "check failed";
        "check failed"
    )]
    #[test_case(
        Outcome::Run { err: "e".into() },
        "run error";
        "run error"
    )]
    #[test_case(
        Outcome::OutputDiff { missing: vec![], unexpected: vec![], with_diff: vec![] },
        "output mismatch";
        "output mismatch"
    )]
    #[test_case(
        Outcome::ExpectedError { expected: "e".into(), actual: "a".into() },
        "wrong error output";
        "expected error"
    )]
    #[test]
    fn format_result_line_includes_correct_reason(outcome: Outcome, expected_reason: &str) {
        let elapsed = Duration::from_millis(100);

        let line = format_result_line("test", &outcome, elapsed, false);

        assert!(
            line.ends_with(&format!(" - {expected_reason}")),
            "expected line to end with ' - {expected_reason}', got: {line}"
        );
    }

    #[test]
    fn format_result_line_timing_sub_second() {
        let outcome = Outcome::Success;
        let elapsed = Duration::from_millis(3);

        let line = format_result_line("test", &outcome, elapsed, false);

        assert!(line.contains("[  0.003s]"), "got: {line}");
    }

    #[test]
    fn format_result_line_timing_multi_second() {
        let outcome = Outcome::Success;
        let elapsed = Duration::from_secs(12) + Duration::from_millis(345);

        let line = format_result_line("test", &outcome, elapsed, false);

        assert!(line.contains("[ 12.345s]"), "got: {line}");
    }

    #[test]
    fn format_result_line_alignment_preserved() {
        let outcome = Outcome::Success;
        let elapsed = Duration::from_millis(1);

        let short = format_result_line("a", &outcome, elapsed, false);
        let long = format_result_line("very_long_test_name", &outcome, elapsed, false);

        // Both should have same prefix up to the name (status + timing)
        let prefix_len = "    PASS [  0.001s] ".len();
        assert_eq!(&short[..prefix_len], &long[..prefix_len]);
    }

    #[test]
    fn normalize_paths_replaces_temp_dir() {
        let temp_dir = Path::new("/tmp/test123");
        let content = "export FILE=\"/tmp/test123/RTF_OUTPUT/file.txt\"";

        let result = normalize_paths(content, temp_dir);

        assert_eq!(result, "export FILE=\"$OUTDIR/RTF_OUTPUT/file.txt\"");
    }

    #[test]
    fn normalize_paths_replaces_multiple_occurrences() {
        let temp_dir = Path::new("/var/folders/abc");
        let content = "A=/var/folders/abc/one\nB=/var/folders/abc/two";

        let result = normalize_paths(content, temp_dir);

        assert_eq!(result, "A=$OUTDIR/one\nB=$OUTDIR/two");
    }

    #[test]
    fn normalize_paths_no_match_unchanged() {
        let temp_dir = Path::new("/tmp/test123");
        let content = "no paths here";

        let result = normalize_paths(content, temp_dir);

        assert_eq!(result, "no paths here");
    }

    #[test]
    fn normalize_paths_empty_content() {
        let temp_dir = Path::new("/tmp/test");

        let result = normalize_paths("", temp_dir);

        assert_eq!(result, "");
    }

    #[test_case("stdout: msg", "msg", ""; "stdout only")]
    #[test_case("stderr: msg", "", "msg"; "stderr only")]
    #[test_case("stdout: out\nstderr: err", "out", "err"; "both")]
    #[test]
    fn check_expected_failure_matching(expected_content: &str, stdout: &str, stderr: &str) {
        let tmp = TempDir::new().unwrap();
        create_failure_test_case(&tmp, "case", expected_content);

        let case = TestCase {
            path: tmp.path().join("case"),
            data: TestCaseData::Failure(expected_content.to_string()),
        };

        let outcome = case.check_expected_failure(stdout.into(), stderr.into());
        assert!(outcome.is_success());
    }

    #[test]
    fn check_expected_failure_mismatch_returns_expected_error() {
        let tmp = TempDir::new().unwrap();
        create_failure_test_case(&tmp, "case", "stdout: expected");

        let case = TestCase {
            path: tmp.path().join("case"),
            data: TestCaseData::Failure("stdout: expected".to_string()),
        };

        let outcome = case.check_expected_failure("actual".into(), String::new());

        match outcome {
            Outcome::ExpectedError { expected, actual } => {
                assert_eq!(expected, "stdout: expected");
                assert_eq!(actual, "stdout: actual");
            }
            _ => panic!("expected ExpectedError, got {outcome:?}"),
        }
    }

    #[test]
    fn check_expected_failure_file_read_error() {
        let tmp = TempDir::new().unwrap();
        // Only create variables.json, not expected-run-error.txt
        tmp.child("case/variables.json").write_str("{}").unwrap();

        let res = TestCase::try_load_all(tmp.path());
        assert!(res.is_err());
    }
}
