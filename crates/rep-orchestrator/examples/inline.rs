use rtf_cli::{
    cli::Variables,
    commands::{get_context_and_check_outdir, load_and_resolve_test_plan_from_local},
};
use rtf_config::{
    DirFile, StableSource,
    context::ResolutionContext,
    formats::TestPlanConfig,
    templating::{Template, TemplateContext},
};
use serde::Serialize;
use std::{
    collections::HashMap,
    env::{self},
    mem::take,
};
use tracing::info;

const INLINED_TEST_PLAN_PATH: &str = "inlined-test-plan.yaml";
const OUTDIR: &str = "output";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let test_plan_path = env::args().nth(1).expect("need a test plan path");
    let (mut ctx, _outdir) = get_context_and_check_outdir(OUTDIR, false)?;
    let variables = Variables {
        var: Vec::new(),
        vars: None,
    };

    info!("loading and resolving test plan");
    let (test_plan, sources) = load_and_resolve_test_plan_from_local(&test_plan_path, &ctx).await?;
    ctx.set_sources(sources);

    extract_relative_files_with_context(test_plan, variables, ctx, OUTDIR).await
}

async fn extract_relative_files_with_context(
    mut test_plan: TestPlanConfig,
    variables: Variables,
    mut ctx: impl ResolutionContext,
    outdir: &str,
) -> anyhow::Result<()> {
    let variable_sources = variables.merge(&mut test_plan, &mut ctx)?;

    info!("creating output directory");
    ctx.create_dir_all(outdir)?;
    let outdir = ctx.canonicalize_path(outdir)?;
    ctx.set_output_path(&outdir);

    let n = test_plan.matrix.n_variants();
    let mut files = HashMap::new();

    for (mut i, (_, mut variant)) in test_plan.try_iter_matrix_variants()?.enumerate() {
        i += 1;

        let variables = take(&mut variant.variables);
        let template_ctx = TemplateContext::new(
            variables,
            StableSource::TestPlan,
            variable_sources.clone(),
            ctx.custom_provider_definitions(),
        );

        info!("extracting relative file providers for matrix variant {i}/{n}");
        variant.try_template(&mut Vec::new(), &StableSource::TestPlan, &template_ctx)?;
        variant.try_extract_relative_files(&mut files, &ctx).await?;
    }

    let yaml_test_plan = test_plan.as_yaml_map()?;
    let mut relative_files: Vec<DirFile> = files
        .into_iter()
        .map(|(path, content)| DirFile { path, content })
        .collect();

    relative_files.sort_by_key(|df| df.path.clone());

    ctx.write(
        outdir.join(INLINED_TEST_PLAN_PATH),
        serde_yaml::to_string(&TestPlanWithFiles {
            test_plan: yaml_test_plan,
            relative_files,
        })?,
    )?;

    Ok(())
}

#[derive(Debug, Serialize)]
struct TestPlanWithFiles {
    test_plan: serde_yaml::Value,
    relative_files: Vec<DirFile>,
}
