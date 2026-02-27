//! Integration tests for resolve plumbing commands
use assert_cmd::cargo::cargo_bin_cmd;
use assert_fs::{TempDir, prelude::*};
use predicates::str::contains;
use std::fs;

#[test]
fn resolve_is_executable() {
    let mut cmd = cargo_bin_cmd!("rtf");
    let res = cmd.arg("resolve").assert();

    res.failure()
        .stderr(contains("Resolve file providers for a config file"));
}

#[test]
fn resolve_scenario_is_executable() {
    let mut cmd = cargo_bin_cmd!("rtf");
    let res = cmd.arg("resolve").arg("scenario").assert();

    res.failure()
        .stderr(contains("required arguments were not provided"));
}

#[test]
fn resolve_script_scenario_creates_expected_output() {
    let tmp = TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    tmp.copy_from("resources/scenarios/valid/script-based", &["**"])
        .unwrap();

    let output_dir = tmp.child("output");

    let mut cmd = cargo_bin_cmd!("rtf");
    let res = cmd
        .env_clear()
        .arg("resolve")
        .arg("scenario")
        .arg(tmp.child("scenario.yaml").path())
        .arg("--outdir")
        .arg(output_dir.path())
        .arg("-v")
        .assert();

    res.success().stderr(contains("done"));

    // Check scenario.env has expected content
    let env_content = fs::read_to_string(output_dir.child("scenario.env").path()).unwrap();
    let out = output_dir.path().to_str().unwrap();
    let expected = r#"export DATA_FILE="{OUT}/providers/data.txt"
export MY_VAR="my_value"
export OUTDIR="{OUT}"
export RTF_OUTPUT="{OUT}/RTF_OUTPUT"
"#
    .replace("{OUT}", out);

    assert_eq!(env_content, expected);

    // Check provider file was written
    let provider_content =
        fs::read_to_string(output_dir.child("providers/data.txt").path()).unwrap();
    assert_eq!(provider_content, "test data content");
}

#[test]
fn resolve_docker_scenario_creates_expected_output() {
    let tmp = TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    tmp.copy_from("resources/scenarios/valid/docker-based", &["**"])
        .unwrap();

    let output_dir = tmp.child("output");

    let mut cmd = cargo_bin_cmd!("rtf");
    let res = cmd
        .env_clear()
        .arg("resolve")
        .arg("scenario")
        .arg(tmp.child("scenario.yaml").path())
        .arg("--outdir")
        .arg(output_dir.path())
        .arg("-v")
        .assert();

    res.success().stderr(contains("done"));

    // Check scenario.env has expected content with HOST paths (not container /output/... paths)
    let env_content = fs::read_to_string(output_dir.child("scenario.env").path()).unwrap();
    let out = output_dir.path().to_str().unwrap();
    let expected = r#"export DATA_FILE="{OUT}/providers/data.txt"
export DOCKER_VAR="docker_value"
export OUTDIR="{OUT}"
export RTF_OUTPUT="{OUT}/RTF_OUTPUT"
"#
    .replace("{OUT}", out);

    assert_eq!(env_content, expected);

    // Check provider file was written
    assert!(output_dir.child("providers/data.txt").exists());
}

#[test]
fn resolve_scenario_with_custom_providers_fails() {
    let tmp = TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    tmp.copy_from("resources/scenarios/invalid/with-custom-providers", &["**"])
        .unwrap();

    let output_dir = tmp.child("output");

    let mut cmd = cargo_bin_cmd!("rtf");
    let res = cmd
        .env_clear()
        .arg("resolve")
        .arg("scenario")
        .arg(tmp.child("scenario.yaml").path())
        .arg("--outdir")
        .arg(output_dir.path())
        .assert();

    res.failure()
        .stderr(contains("custom_providers are not supported"));
}

#[test]
fn resolve_scenario_with_missing_variables_fails() {
    let tmp = TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    tmp.copy_from("resources/scenarios/invalid/missing-variables", &["**"])
        .unwrap();

    let output_dir = tmp.child("output");

    let mut cmd = cargo_bin_cmd!("rtf");
    let res = cmd
        .env_clear()
        .arg("resolve")
        .arg("scenario")
        .arg(tmp.child("scenario.yaml").path())
        .arg("--outdir")
        .arg(output_dir.path())
        .assert();

    res.failure()
        .stderr(contains("Templating failed"))
        .stderr(contains("Unknown templating variable"))
        .stderr(contains("undefined_var"));
}

#[test]
fn resolve_environment_is_executable() {
    let mut cmd = cargo_bin_cmd!("rtf");
    let res = cmd.arg("resolve").arg("environment").assert();

    res.failure()
        .stderr(contains("required arguments were not provided"));
}

#[test]
fn resolve_script_environment_creates_expected_output() {
    let tmp = TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    tmp.copy_from("resources/environments/valid/script-based", &["**"])
        .unwrap();

    let output_dir = tmp.child("output");

    let mut cmd = cargo_bin_cmd!("rtf");
    let res = cmd
        .env_clear()
        .arg("resolve")
        .arg("environment")
        .arg(tmp.child("environment.yaml").path())
        .arg("--outdir")
        .arg(output_dir.path())
        .arg("-v")
        .assert();

    res.success().stderr(contains("done"));

    assert!(output_dir.child("setup").exists());
    assert!(output_dir.child("setup/providers").exists());
    assert!(output_dir.child("setup/setup.env").exists());

    assert!(output_dir.child("teardown").exists());
    assert!(output_dir.child("teardown/providers").exists());
    assert!(output_dir.child("teardown/teardown.env").exists());

    let setup_env = fs::read_to_string(output_dir.child("setup/setup.env").path()).unwrap();
    let setup_dir = output_dir.child("setup");
    let setup_out = setup_dir.path().to_str().unwrap();
    let expected_setup = format!(
        r#"export OUTDIR="{setup_out}"
export RTF_OUTPUT="{setup_out}/RTF_OUTPUT"
export SETUP_CONFIG="{setup_out}/providers/setup-config.txt"
export SETUP_VAR="setup_value"
"#
    );
    assert_eq!(setup_env, expected_setup);

    let teardown_env =
        fs::read_to_string(output_dir.child("teardown/teardown.env").path()).unwrap();
    let teardown_dir = output_dir.child("teardown");
    let teardown_out = teardown_dir.path().to_str().unwrap();
    let expected_teardown = format!(
        r#"export OUTDIR="{teardown_out}"
export RTF_OUTPUT="{teardown_out}/RTF_OUTPUT"
export TEARDOWN_CONFIG="{teardown_out}/providers/teardown-config.txt"
export TEARDOWN_VAR="teardown_value"
"#
    );
    assert_eq!(teardown_env, expected_teardown);

    let setup_config =
        fs::read_to_string(output_dir.child("setup/providers/setup-config.txt").path()).unwrap();
    assert_eq!(setup_config, "setup config content");

    let teardown_config = fs::read_to_string(
        output_dir
            .child("teardown/providers/teardown-config.txt")
            .path(),
    )
    .unwrap();
    assert_eq!(teardown_config, "teardown config content");
}

#[test]
fn resolve_docker_compose_environment_creates_expected_output() {
    let tmp = TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    tmp.copy_from("resources/environments/valid/docker-compose-based", &["**"])
        .unwrap();

    let output_dir = tmp.child("output");

    let mut cmd = cargo_bin_cmd!("rtf");
    let res = cmd
        .env_clear()
        .arg("resolve")
        .arg("environment")
        .arg(tmp.child("environment.yaml").path())
        .arg("--outdir")
        .arg(output_dir.path())
        .arg("-v")
        .assert();

    res.success().stderr(contains("done"));

    assert!(output_dir.child("setup").exists());
    assert!(output_dir.child("setup/providers").exists());
    assert!(output_dir.child("setup/setup.env").exists());

    assert!(output_dir.child("teardown").exists());
    assert!(output_dir.child("teardown/providers").exists());
    assert!(output_dir.child("teardown/teardown.env").exists());

    assert!(output_dir.child("setup/compose-files.txt").exists());
    let compose_files =
        fs::read_to_string(output_dir.child("setup/compose-files.txt").path()).unwrap();
    assert!(compose_files.contains("docker-compose.yaml"));

    let setup_env = fs::read_to_string(output_dir.child("setup/setup.env").path()).unwrap();
    assert!(setup_env.contains("COMPOSE_FILES="));
    assert!(setup_env.contains("compose-files.txt"));
    assert!(setup_env.contains("COMPOSE_VAR=\"compose_value\""));
    assert!(setup_env.contains("CONFIG_FILE="));

    let teardown_env =
        fs::read_to_string(output_dir.child("teardown/teardown.env").path()).unwrap();
    assert_eq!(
        teardown_env,
        "export COMPOSE_PROJECT_NAME=\"test-project\"\n"
    );

    assert!(
        output_dir
            .child("setup/providers/docker-compose.yaml")
            .exists()
    );
}

#[test]
fn resolve_environment_with_custom_providers_fails() {
    let tmp = TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    tmp.copy_from(
        "resources/environments/invalid/with-custom-providers",
        &["**"],
    )
    .unwrap();

    let output_dir = tmp.child("output");

    let mut cmd = cargo_bin_cmd!("rtf");
    let res = cmd
        .env_clear()
        .arg("resolve")
        .arg("environment")
        .arg(tmp.child("environment.yaml").path())
        .arg("--outdir")
        .arg(output_dir.path())
        .assert();

    res.failure()
        .stderr(contains("custom_providers are not supported"));
}

#[test]
fn resolve_environment_with_missing_variables_fails() {
    let tmp = TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    tmp.copy_from("resources/environments/invalid/missing-variables", &["**"])
        .unwrap();

    let output_dir = tmp.child("output");

    let mut cmd = cargo_bin_cmd!("rtf");
    let res = cmd
        .env_clear()
        .arg("resolve")
        .arg("environment")
        .arg(tmp.child("environment.yaml").path())
        .arg("--outdir")
        .arg(output_dir.path())
        .assert();

    res.failure()
        .stderr(contains("Templating failed"))
        .stderr(contains("Unknown templating variable"))
        .stderr(contains("undefined_var"));
}

#[test]
fn resolve_docker_compose_multi_file_writes_paths_to_file() {
    let tmp = TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    tmp.copy_from(
        "resources/environments/valid/docker-compose-multi-file",
        &["**"],
    )
    .unwrap();

    let output_dir = tmp.child("output");

    let mut cmd = cargo_bin_cmd!("rtf");
    let res = cmd
        .env_clear()
        .arg("resolve")
        .arg("environment")
        .arg(tmp.child("environment.yaml").path())
        .arg("--outdir")
        .arg(output_dir.path())
        .arg("-v")
        .assert();

    res.success().stderr(contains("done"));

    assert!(output_dir.child("setup/compose-files.txt").exists());

    let compose_files_content =
        fs::read_to_string(output_dir.child("setup/compose-files.txt").path()).unwrap();
    let paths: Vec<&str> = compose_files_content.lines().collect();

    assert_eq!(paths.len(), 2, "Should have exactly 2 compose files");

    for path in &paths {
        assert!(
            std::path::Path::new(path).exists(),
            "Compose file should exist: {path}"
        );
    }

    let setup_env = fs::read_to_string(output_dir.child("setup/setup.env").path()).unwrap();
    assert!(
        setup_env.contains("COMPOSE_FILES="),
        "COMPOSE_FILES should be in setup.env"
    );
    assert!(
        setup_env.contains("compose-files.txt"),
        "COMPOSE_FILES should point to compose-files.txt"
    );

    assert!(output_dir.child("setup/providers/base.yaml").exists());
    assert!(output_dir.child("setup/providers/override.yaml").exists());
}

#[test]
fn resolve_docker_compose_no_project_name_uses_environment_name() {
    let tmp = TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    tmp.copy_from(
        "resources/environments/valid/docker-compose-no-project-name",
        &["**"],
    )
    .unwrap();

    let output_dir = tmp.child("output");

    let mut cmd = cargo_bin_cmd!("rtf");
    let res = cmd
        .env_clear()
        .arg("resolve")
        .arg("environment")
        .arg(tmp.child("environment.yaml").path())
        .arg("--outdir")
        .arg(output_dir.path())
        .arg("-v")
        .assert();

    res.success().stderr(contains("done"));

    let teardown_env =
        fs::read_to_string(output_dir.child("teardown/teardown.env").path()).unwrap();

    assert_eq!(
        teardown_env,
        "export COMPOSE_PROJECT_NAME=\"fallback-project-name\"\n"
    );
}

#[test]
fn resolve_scenario_with_existing_outdir_fails() {
    let tmp = TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    tmp.copy_from("resources/scenarios/valid/script-based", &["**"])
        .unwrap();

    let output_dir = tmp.child("output");
    output_dir.create_dir_all().unwrap();
    output_dir
        .child("existing-file.txt")
        .write_str("content")
        .unwrap();

    let mut cmd = cargo_bin_cmd!("rtf");
    let res = cmd
        .env_clear()
        .arg("resolve")
        .arg("scenario")
        .arg(tmp.child("scenario.yaml").path())
        .arg("--outdir")
        .arg(output_dir.path())
        .assert();

    res.failure()
        .stderr(contains("already exists and is non-empty"));
}

#[test]
fn resolve_scenario_with_existing_outdir_and_force_succeeds() {
    let tmp = TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    tmp.copy_from("resources/scenarios/valid/script-based", &["**"])
        .unwrap();

    let output_dir = tmp.child("output");
    output_dir.create_dir_all().unwrap();
    output_dir
        .child("existing-file.txt")
        .write_str("content")
        .unwrap();

    let mut cmd = cargo_bin_cmd!("rtf");
    let res = cmd
        .env_clear()
        .arg("resolve")
        .arg("scenario")
        .arg(tmp.child("scenario.yaml").path())
        .arg("--outdir")
        .arg(output_dir.path())
        .arg("--force")
        .arg("-v")
        .assert();

    res.success().stderr(contains("done"));
}

#[test]
fn resolve_environment_with_existing_outdir_fails() {
    let tmp = TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    tmp.copy_from("resources/environments/valid/script-based", &["**"])
        .unwrap();

    let output_dir = tmp.child("output");
    output_dir.create_dir_all().unwrap();
    output_dir
        .child("existing-file.txt")
        .write_str("content")
        .unwrap();

    let mut cmd = cargo_bin_cmd!("rtf");
    let res = cmd
        .env_clear()
        .arg("resolve")
        .arg("environment")
        .arg(tmp.child("environment.yaml").path())
        .arg("--outdir")
        .arg(output_dir.path())
        .assert();

    res.failure()
        .stderr(contains("already exists and is non-empty"));
}

#[test]
fn resolve_environment_with_existing_outdir_and_force_succeeds() {
    let tmp = TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    tmp.copy_from("resources/environments/valid/script-based", &["**"])
        .unwrap();

    let output_dir = tmp.child("output");
    output_dir.create_dir_all().unwrap();
    output_dir
        .child("existing-file.txt")
        .write_str("content")
        .unwrap();

    let mut cmd = cargo_bin_cmd!("rtf");
    let res = cmd
        .env_clear()
        .arg("resolve")
        .arg("environment")
        .arg(tmp.child("environment.yaml").path())
        .arg("--outdir")
        .arg(output_dir.path())
        .arg("--force")
        .arg("-v")
        .assert();

    res.success().stderr(contains("done"));
}

#[test]
fn resolve_docker_compose_setup_env_has_complete_content() {
    let tmp = TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    tmp.copy_from(
        "resources/environments/valid/docker-compose-multi-file",
        &["**"],
    )
    .unwrap();

    let output_dir = tmp.child("output");

    let mut cmd = cargo_bin_cmd!("rtf");
    let res = cmd
        .env_clear()
        .arg("resolve")
        .arg("environment")
        .arg(tmp.child("environment.yaml").path())
        .arg("--outdir")
        .arg(output_dir.path())
        .arg("-v")
        .assert();

    res.success().stderr(contains("done"));

    let setup_env = fs::read_to_string(output_dir.child("setup/setup.env").path()).unwrap();
    let setup_dir = output_dir.child("setup");
    let setup_out = setup_dir.path().to_str().unwrap();
    let expected_setup = format!(
        r#"export APP_CONFIG="{setup_out}/providers/app-config.json"
export APP_ENV="test"
export COMPOSE_FILES="{setup_out}/compose-files.txt"
export OUTDIR="{setup_out}"
export RTF_OUTPUT="{setup_out}/RTF_OUTPUT"
"#
    );
    assert_eq!(setup_env, expected_setup);

    let app_config =
        fs::read_to_string(output_dir.child("setup/providers/app-config.json").path()).unwrap();
    assert_eq!(app_config, "{\"key\": \"value\"}");
}
