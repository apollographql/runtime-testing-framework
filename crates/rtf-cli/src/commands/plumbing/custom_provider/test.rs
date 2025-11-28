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
            let detail = outcome.detail();
            if !detail.is_empty() {
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
                return Err(io_err(io::ErrorKind::NotFound, &expected_output_dir));
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
        let actual = output_files(&output_dir)?;
        let expected = output_files(&self.path.join(EXPECTED_RUN_OUTPUT_DIR))?;

        Ok(compare_outputs(&actual, &expected))
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
    #[test]
    fn outcome_is_success(outcome: Outcome, expected: bool) {
        assert_eq!(outcome.is_success(), expected);
    }

    #[test_case(Outcome::Success, "passed"; "success")]
    #[test_case(Outcome::Template { err: "e".into() }, "template error"; "template")]
    #[test_case(Outcome::Check { err: "e".into() }, "check failed"; "check")]
    #[test_case(Outcome::Run { err: "e".into() }, "run error"; "run")]
    #[test_case(Outcome::OutputDiff { missing: vec![], unexpected: vec![], with_diff: vec![] }, "output mismatch"; "output diff")]
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

    fn create_valid_test_case(tmp: &TempDir, name: &str) {
        tmp.child(format!("{name}/variables.json"))
            .write_str("{}")
            .unwrap();
        tmp.child(format!("{name}/expected-run-output"))
            .create_dir_all()
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
        &[TestAsset::File("case-a/variables.json", "{}")],
        None,
        io::ErrorKind::NotFound;
        "missing expected output"
    )]
    #[test_case(
        &[TestAsset::File("case-a/variables.json", "{}"), TestAsset::File("case-a/expected-run-output", "")],
        None,
        io::ErrorKind::NotADirectory;
        "expected output is file"
    )]
    #[test]
    fn try_load_all_error(assets: &[TestAsset], subpath: Option<&str>, expected: io::ErrorKind) {
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
        let res = TestCase::try_load_all(&path);
        assert!(res.is_err());
        assert_eq!(
            res.unwrap_err().downcast_ref::<io::Error>().unwrap().kind(),
            expected
        );
    }
}
