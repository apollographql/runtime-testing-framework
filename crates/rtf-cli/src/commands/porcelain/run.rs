use rtf_config::{context::Context, formats::TestPlanConfig, templating::Scalar};
use std::{
    collections::HashMap,
    env::{current_dir, set_current_dir},
    fs::create_dir_all,
    mem::take,
    path::PathBuf,
};
use tracing::info;

pub async fn validate_and_run_test_plan(path: &str, out_dir: &str) -> anyhow::Result<()> {
    info!("loading and resolving test plan");
    let mut test_plan = TestPlanConfig::try_load_and_resolve_from_path(path).await?;
    info!("checking if templating will work");
    test_plan.validate_templating_will_work()?;

    // prepare context and output directory
    info!("setting up context and output directory");
    let mut values = take(&mut test_plan.values);
    let full_path = PathBuf::from(path).canonicalize()?;
    let config_dir = full_path.parent().unwrap().to_path_buf();

    // we ensure that we are running from the directory containing the test plan so that relative
    // paths within config files are correct
    let execution_dir = current_dir()?;
    set_current_dir(&config_dir)?;

    // output directories are created relative to the directory we were run from
    let out_dir = execution_dir.join(out_dir);

    if out_dir.exists() {
        if !out_dir.is_dir() {
            anyhow::bail!("{} is not a directory", out_dir.display());
        } else if out_dir.read_dir()?.next().is_some() {
            anyhow::bail!("{} already exists and is non-empty", out_dir.display());
        }
    }

    info!("creating output directory");
    create_dir_all(&out_dir)?;
    let ctx = Context::new(&config_dir);

    info!("resolving environment setup");
    test_plan.try_resolve_envrionment_setup(&values)?;
    info!("executing environment setup");
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

    // TODO: errors need combining and returning together
    info!("resolving scenario and environment teardown commands");
    test_plan.try_resolve_scenario(&values)?;
    test_plan.try_resolve_envrionment_teardown(&values)?;

    info!("executing scenario");
    test_plan
        .scenario
        .command
        .run_providers_and_execute(&out_dir, &ctx)
        .await?;

    info!("executing environment teardown");
    test_plan
        .environment
        .teardown
        .run_providers_and_execute(&out_dir, &ctx)
        .await?;

    Ok(())
}
