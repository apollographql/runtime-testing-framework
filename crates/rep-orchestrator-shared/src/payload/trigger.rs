use crate::{payload::SourceKeyedArrayMap, test_plan::RepTestPlan};
use rtf_config::{
    StableSource,
    context::ResolutionContext,
    formats::{CustomProviderDefinition, Sources},
    run::RunProviders,
    templating::{Template, TemplateContext},
};
use rtf_core::variables::Variables;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, mem::take, sync::Arc};
use tracing::info;

#[derive(Debug, Deserialize, Serialize)]
pub struct TriggerPayload {
    pub test_plan: RepTestPlan,
    pub relative_files: SourceKeyedArrayMap<String>,
    pub custom_providers: SourceKeyedArrayMap<CustomProviderDefinition>,
}

impl TriggerPayload {
    pub async fn prepare(
        mut test_plan: RepTestPlan,
        sources: Sources,
        variables: Variables,
        mut ctx: impl ResolutionContext,
    ) -> anyhow::Result<TriggerPayload> {
        let (variable_sources, vars_file_src) = variables.merge(&mut test_plan, &ctx)?;
        ctx.set_sources(sources.with_variables_file(vars_file_src));

        let n = test_plan.matrix.n_variants();
        let mut files = HashMap::new();

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
            try_extract_relative_files(&variant, &mut files, &ctx).await?;
        }

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

        Ok(Self {
            test_plan,
            relative_files: SourceKeyedArrayMap::from_data(files),
            custom_providers: SourceKeyedArrayMap::from_data(raw_cps),
        })
    }
}

async fn try_extract_relative_files(
    test_plan: &RepTestPlan,
    files: &mut HashMap<(StableSource, String), String>,
    ctx: &impl ResolutionContext,
) -> anyhow::Result<()> {
    test_plan
        .environment
        .execution
        .try_extract_relative_files(files, ctx)
        .await?;
    test_plan
        .scenario
        .execution
        .try_extract_relative_files(files, ctx)
        .await?;

    Ok(())
}
