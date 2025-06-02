use rtf_config::{context::Context, formats::TestPlanConfig, templating::Scalar};
use std::{collections::HashMap, fs::create_dir_all, mem::take, path::PathBuf};

pub async fn validate_and_run_test_plan(path: &str, out_dir: &str) -> anyhow::Result<()> {
    let mut test_plan = TestPlanConfig::try_load_and_resolve_from_path(path).await?;
    test_plan.validate_templating_will_work()?;

    // prepare context and output directory
    let mut values = take(&mut test_plan.values);
    let full_path = PathBuf::from(path).canonicalize()?;
    let config_dir = full_path.parent().unwrap().to_path_buf();
    let out_dir = config_dir.join(out_dir);

    if out_dir.exists() {
        if !out_dir.is_dir() {
            anyhow::bail!("{} is not a directory", out_dir.display());
        } else if out_dir.read_dir()?.next().is_some() {
            anyhow::bail!("{} already exists and is non-empty", out_dir.display());
        }
    }

    create_dir_all(&out_dir)?;
    let ctx = Context::new(&config_dir);

    // try to resolve and run the environment setup
    test_plan.try_resolve_envrionment_setup(&values)?;
    let raw_output = test_plan
        .environment
        .setup
        .command
        .run_providers_and_execute(&out_dir, &ctx)
        .await?;

    // TODO: validate that this matches what was declared by the setup command section
    // and filter to only make use of the declared values
    let setup_provides: HashMap<String, Scalar> = serde_json::from_str(&raw_output)?;
    values.extend(setup_provides);

    // resolve the scenario and teardown
    // TODO: errors need combining
    test_plan.try_resolve_scenario(&values)?;
    test_plan.try_resolve_envrionment_teardown(&values)?;

    // run the scenario
    test_plan
        .scenario
        .command
        .run_providers_and_execute(&out_dir, &ctx)
        .await?;

    // run the environment teardown
    test_plan
        .environment
        .teardown
        .run_providers_and_execute(&out_dir, &ctx)
        .await?;

    Ok(())
}
