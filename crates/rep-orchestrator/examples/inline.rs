use rtf_cli::{
    cli::Variables,
    commands::{get_context_and_check_outdir, load_and_resolve_test_plan_from_local},
};
use rtf_config::{
    DirFile, SourceDir,
    context::ResolutionContext,
    formats::TestPlanConfig,
    templating::{Template, TemplateContext},
};
use serde::Serialize;
use std::{
    collections::HashMap,
    env::{self, current_dir},
    mem::take,
    path::PathBuf,
};
use tracing::info;

const INLINED_TEST_PLAN_PATH: &str = "inlined-test-plan.yaml";
const OUTDIR: &str = "output";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let test_plan_path = env::args().nth(1).expect("need a test plan path");
    let cwd = current_dir()?;
    let (ctx, _outdir) = get_context_and_check_outdir(OUTDIR, false)?;
    let variables = Variables {
        var: Vec::new(),
        vars: None,
    };

    info!("loading and resolving test plan");
    let test_plan = load_and_resolve_test_plan_from_local(&test_plan_path, &ctx).await?;

    extract_relative_files_with_context(test_plan, variables, ctx, cwd, OUTDIR).await
}

async fn extract_relative_files_with_context(
    mut test_plan: TestPlanConfig,
    variables: Variables,
    mut ctx: impl ResolutionContext,
    cwd: PathBuf,
    outdir: &str,
) -> anyhow::Result<()> {
    let variable_sources = variables.merge(&mut test_plan, &SourceDir::local(cwd), &mut ctx)?;

    info!("creating output directory");
    ctx.create_dir_all(outdir)?;
    let outdir = ctx.canonicalize_path(outdir)?;
    ctx.set_output_path(&outdir);

    let n = test_plan.matrix.n_variants();
    let mut files = HashMap::new();

    for (mut i, (_, mut variant)) in test_plan.try_iter_matrix_variants()?.enumerate() {
        i += 1;

        let variables = take(&mut variant.variables);
        let source = variant.sources.test_plan().clone();
        let template_ctx = TemplateContext::new(
            variables,
            variant.sources.test_plan().clone(),
            variable_sources.clone(),
            variant.sources.custom_providers(),
        );

        info!("extracting relative file providers for matrix variant {i}/{n}");
        variant.try_template(&mut Vec::new(), &source, &template_ctx)?;
        variant.try_extract_relative_files(&mut files, &ctx).await?;
    }

    let yaml_test_plan = test_plan.as_yaml_map_without_sources()?;
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
