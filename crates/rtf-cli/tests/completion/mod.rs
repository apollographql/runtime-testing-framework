use assert_cmd::cargo::cargo_bin_cmd;
use predicates::str::contains;

#[test]
fn is_executable() {
    cargo_bin_cmd!("rtf")
        .args(["completion", "-h"])
        .assert()
        .stdout(contains("Usage: rtf completion [OPTIONS]"));
}

#[test]
fn bash_succeeds() {
    cargo_bin_cmd!("rtf")
        .args(["completion", "-s", "bash"])
        .assert()
        .success();
}

#[test]
fn elvish_succeeds() {
    cargo_bin_cmd!("rtf")
        .args(["completion", "-s", "elvish"])
        .assert()
        .success();
}

#[test]
fn fish_succeeds() {
    cargo_bin_cmd!("rtf")
        .args(["completion", "-s", "fish"])
        .assert()
        .success();
}

#[test]
fn powershell_succeeds() {
    cargo_bin_cmd!("rtf")
        .args(["completion", "-s", "powershell"])
        .assert()
        .success();
}

#[test]
fn zsh_succeeds() {
    cargo_bin_cmd!("rtf")
        .args(["completion", "-s", "zsh"])
        .assert()
        .success();
}

// We use clap_complete's Shell::from_env method to attempt to infer the shell we are running under
// via the SHELL environment variable when the -s/--shell flag is omitted.
//
//   https://docs.rs/clap_complete/4.5.65/clap_complete/aot/enum.Shell.html#method.from_env

#[test]
fn infer_from_env_known_succeeds() {
    cargo_bin_cmd!("rtf")
        .args(["completion"])
        .env("SHELL", "/bin/bash")
        .assert()
        .success();
}

#[test]
fn infer_from_env_unknown_errors() {
    cargo_bin_cmd!("rtf")
        .args(["completion"])
        .env("SHELL", "/usr/bin/not-a-real-shell")
        .assert()
        .failure()
        .stderr(contains(
            "Unable to determine current shell, please specify using the --shell flag",
        ));
}
