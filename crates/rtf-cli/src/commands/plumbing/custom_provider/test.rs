//! Run validation tests for a custom provider
use crate::commands::{get_context, plumbing::custom_provider::load_definition};
use anyhow::{Context as _, anyhow};
use rtf_config::context::ResolutionContext;
use rtf_core::custom_provider::{Outcome, TestSuite};
use std::{
    io::{IsTerminal, stdout},
    time::{Duration, Instant},
};
use tracing::{error, warn};

const ANSI_GREEN: &str = "\x1b[32m";
const ANSI_RED: &str = "\x1b[31m";
const ANSI_RESET: &str = "\x1b[0m";
const TEST_CASES_DIR: &str = "test-cases";

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

    let suite = TestSuite::try_load(&ctx.canonicalize_path(test_cases_dir)?)?;

    if suite.is_empty() && error_on_empty {
        error!("No test cases found");
        return Err(anyhow!("No test cases found"));
    }

    let total = suite.len();
    let total_start = Instant::now();
    let is_tty = stdout().is_terminal();

    let mut outcomes: Vec<Outcome> = Vec::with_capacity(total);
    let rx = suite.spawn_cases(definition, source, get_context);

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
    use simple_test_case::test_case;

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
}
