use rtf_cli::{
    cli::Variables,
    commands::{get_context_and_check_outdir, load_and_resolve_test_plan_from_local},
};
use rtf_config::{
    StableSource,
    context::ResolutionContext,
    formats::{CustomProviderDefinition, Sources, TestPlanConfig},
    templating::{Template, TemplateContext},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    env::{self},
    mem::take,
    sync::Arc,
};
use tracing::info;

const REP_TEST_PLAN_PATH: &str = "rep-test-plan.yaml";
const OUTDIR: &str = "output";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let test_plan_path = env::args().nth(1).expect("need a test plan path");
    let (ctx, _outdir) = get_context_and_check_outdir(OUTDIR, false)?;
    let variables = Variables {
        var: Vec::new(),
        vars: None,
    };

    info!("loading and resolving test plan");
    let (test_plan, sources) = load_and_resolve_test_plan_from_local(&test_plan_path, &ctx).await?;
    extract_relative_files_with_context(test_plan, sources, variables, ctx, OUTDIR).await
}

async fn extract_relative_files_with_context(
    mut test_plan: TestPlanConfig,
    sources: Sources,
    variables: Variables,
    mut ctx: impl ResolutionContext,
    outdir: &str,
) -> anyhow::Result<()> {
    let (variable_sources, vars_file_src) = variables.merge(&mut test_plan, &ctx)?;
    ctx.set_sources(sources.with_variables_file(vars_file_src));

    info!("creating output directory");
    ctx.create_dir_all(outdir)?;
    let outdir = ctx.canonicalize_path(outdir)?;
    ctx.set_output_path(&outdir);

    let n = test_plan.matrix.n_variants();
    let mut files = HashMap::new();

    // Extract file providers
    for (mut i, (_, mut variant)) in test_plan.try_iter_matrix_variants()?.enumerate() {
        i += 1;

        let variables = take(&mut variant.variables);
        let template_ctx = TemplateContext::new(
            variables,
            variable_sources.clone(),
            ctx.custom_provider_definitions(),
        );

        info!("extracting relative file providers for matrix variant {i}/{n}");
        variant.try_template(&mut Vec::new(), &StableSource::TestPlan, &template_ctx)?;
        variant.try_extract_relative_files(&mut files, &ctx).await?;
    }

    // Extract custom provider definitions
    let custom_providers = Arc::unwrap_or_clone(ctx.custom_provider_definitions());
    let mut raw_cps = HashMap::new();
    for (k, def) in custom_providers.test_plan.into_iter() {
        raw_cps.insert((StableSource::TestPlan, k), def);
    }
    for (k, def) in custom_providers.environment.into_iter() {
        raw_cps.insert((StableSource::Environment, k), def);
    }
    for (k, def) in custom_providers.scenario.into_iter() {
        raw_cps.insert((StableSource::Scenario, k), def);
    }

    ctx.write(
        outdir.join(REP_TEST_PLAN_PATH),
        serde_yaml::to_string(&RepTestPlan {
            test_plan,
            relative_files: SourceKeyedMap::from_data(files),
            custom_providers: SourceKeyedMap::from_data(raw_cps),
        })?,
    )?;

    Ok(())
}

#[derive(Debug, Deserialize, Serialize)]
struct RepTestPlan {
    test_plan: TestPlanConfig,
    relative_files: SourceKeyedMap<String>,
    custom_providers: SourceKeyedMap<CustomProviderDefinition>,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
struct SourceKey {
    src: StableSource,
    k: String,
    index: usize,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
struct SourceKeyedMap<T> {
    keys: Vec<SourceKey>,
    data: Vec<T>,
}

impl<T> SourceKeyedMap<T>
where
    T: PartialEq,
{
    fn from_data(raw: HashMap<(StableSource, String), T>) -> Self {
        let mut keys = Vec::with_capacity(raw.len());
        let mut data = Vec::with_capacity(raw.len());

        let mut raw: Vec<_> = raw.into_iter().collect();
        raw.sort_unstable_by_key(|(k, _)| k.clone());

        for ((src, k), t) in raw.into_iter() {
            let index = match data.iter().position(|known| known == &t) {
                Some(i) => i,
                None => {
                    let i = data.len();
                    data.push(t);
                    i
                }
            };

            keys.push(SourceKey { src, k, index });
        }

        Self { keys, data }
    }

    // fn into_map_and_data(self) -> (HashMap<(StableSource, String), usize>, Vec<T>) {
    //     (
    //         self.keys
    //             .into_iter()
    //             .map(|SourceKey { src, k, index }| ((src, k), index))
    //             .collect(),
    //         self.data,
    //     )
    // }
}
