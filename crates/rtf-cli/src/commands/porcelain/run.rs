use crate::{
    cli::Values,
    commands::{get_context_and_check_outdir, load_and_resolve_test_plan},
};
use anyhow::anyhow;
use rtf_config::{
    checks::{self, Check},
    context::ResolutionContext,
    formats::TestPlanConfig,
    providers::file::Source,
    templating::{self, TemplateValues},
};
use std::{
    collections::HashMap,
    env::current_dir,
    mem::take,
    path::{Path, PathBuf},
};
use tracing::info;

const VALUES_PATH: &str = "test-plan-values.json";
const RESOLVED_TP_PATH: &str = "resolved-test-plan.yaml";

pub async fn check_and_run_local_test_plan(
    config_file_path: &str,
    values: Values,
    out_dir: &str,
) -> anyhow::Result<()> {
    let (ctx, out_dir) = get_context_and_check_outdir(out_dir)?;
    let cwd = current_dir()?;

    info!("loading and resolving test plan");
    let test_plan = load_and_resolve_test_plan(config_file_path, &ctx).await?;

    check_and_run_test_plan_with_context(test_plan, values, &out_dir, cwd, ctx).await
}

pub async fn check_and_run_github_test_plan(
    org_repo_path: String,
    git_ref: Option<String>,
    values: Values,
    out_dir: &str,
) -> anyhow::Result<()> {
    let (ctx, out_dir) = get_context_and_check_outdir(out_dir)?;
    let cwd = current_dir()?;

    let (org, repo_and_path) = org_repo_path.split_once('/').ok_or(anyhow!(
        "invalid GitHub uri: \"{org_repo_path}\" - GitHub uri must be in format ORG/REPO/PATH"
    ))?;
    let (repo, path) = repo_and_path.split_once('/').ok_or(anyhow!(
        "invalid GitHub uri: \"{org_repo_path}\" - GitHub uri must be in format ORG/REPO/PATH"
    ))?;

    info!("fetching and resolving test plan from GitHub");
    let test_plan =
        TestPlanConfig::try_load_and_resolve_from_github(org, repo, path, git_ref, &ctx).await?;

    check_and_run_test_plan_with_context(test_plan, values, &out_dir, cwd, ctx).await
}

async fn check_and_run_test_plan_with_context(
    mut test_plan: TestPlanConfig,
    values: Values,
    out_dir: &Path,
    cwd: PathBuf,
    mut ctx: impl ResolutionContext,
) -> anyhow::Result<()> {
    let override_sources =
        values.merge(&mut test_plan, &Source::local(cwd.join("cli")), &mut ctx)?;

    info!("checking if templating will work");
    test_plan.check_templating_will_work()?;

    info!("creating output directory");
    ctx.create_dir_all(out_dir)?;
    let out_dir = ctx.canonicalize_path(out_dir)?;

    if let Source::Local { abs_path } = test_plan.sources.test_plan() {
        let config_dir = ctx.dir_containing(abs_path);
        ctx.set_current_dir(config_dir)?;
    }

    if test_plan.matrix.is_empty() {
        info!("executing test plan");
        return run_one(test_plan, &out_dir, &override_sources, &mut ctx).await;
    }

    let n = test_plan.matrix.n_variants();

    for (mut i, (name, tp)) in test_plan.try_iter_matrix_variants()?.enumerate() {
        i += 1;
        ctx.set_values(&tp.values);
        let sub_dir = out_dir.join(name);
        info!("creating output directory for matrix variant {i}/{n}");
        ctx.create_dir_all(&sub_dir)?;

        info!("executing test plan {i}/{n}");
        run_one(tp, &sub_dir, &override_sources, &mut ctx).await?;
    }

    Ok(())
}

async fn run_one(
    mut test_plan: TestPlanConfig,
    out_dir: &Path,
    override_sources: &HashMap<String, Source>,
    ctx: &mut impl ResolutionContext,
) -> anyhow::Result<()> {
    info!("templating environment setup");
    let values = take(&mut test_plan.values);
    let mut template_values = TemplateValues::new(
        values,
        test_plan.sources.test_plan().clone(),
        override_sources.clone(),
    );
    test_plan.try_template_environment_setup(&template_values)?;

    info!("checking environment setup");
    test_plan
        .environment
        .setup
        .command
        .try_check(&mut Vec::new(), ctx)?;

    info!("executing environment setup");
    let setup_provides = test_plan.run_environment_setup(out_dir, ctx).await?;
    template_values.extend(Source::local(out_dir), setup_provides);

    info!("templating scenario and environment teardown commands");
    let mut builder =
        templating::ErrorBuilder::from(test_plan.try_template_scenario(&template_values));
    builder.append(test_plan.try_template_environment_teardown(&template_values));
    builder.into_result(())?;

    info!("checking scenario and environment teardown commands");
    let mut builder =
        checks::ErrorBuilder::from(test_plan.scenario.command.try_check(&mut Vec::new(), ctx));
    builder.append(
        test_plan
            .environment
            .teardown
            .try_check(&mut Vec::new(), ctx),
    );
    builder.into_result(())?;

    info!("executing scenario");
    test_plan.run_scenario(out_dir, ctx).await?;

    info!("executing environment teardown");
    test_plan.run_environment_teardown(out_dir, ctx).await?;

    info!("writing out resolved test plan and values");
    ctx.write(
        out_dir.join(VALUES_PATH),
        serde_json::to_string_pretty(template_values.inner())?,
    )?;
    ctx.write(
        out_dir.join(RESOLVED_TP_PATH),
        serde_yaml::to_string(&test_plan)?,
    )?;

    info!("done");

    Ok(())
}
