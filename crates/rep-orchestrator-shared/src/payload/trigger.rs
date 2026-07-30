use crate::{payload::SourceKeyedArrayMap, test_plan::RepTestPlan};
use rtf_config::{
    SourceDir, StableSource,
    context::ResolutionContext,
    formats::{self, CustomProviderDefinition, Sources},
    providers,
    run::RunProviders,
    templating::{self, CustomProviderDefinitions, Template, TemplateContext},
};
use rtf_core::variables::{self, ParsedVariables, ScalarOrArray};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, mem::take, sync::Arc};
use tracing::info;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
#[allow(clippy::large_enum_variant)]
pub enum TriggerPayload {
    /// An inlined test plan prepared via the rtf CLI
    Prepared(PreparedPayload),
    /// Details for pulling a test plan from GitHub
    GitHub(GitHubPayload),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GitHubPayload {
    pub org: String,
    pub repo: String,
    pub path: String,
    #[serde(default, rename = "ref")]
    pub git_ref: Option<String>,
    #[serde(default)]
    pub variables: Option<HashMap<String, ScalarOrArray>>,
}

impl GitHubPayload {
    pub async fn into_prepared(
        self,
        ctx: impl ResolutionContext,
    ) -> Result<PreparedPayload, PrepareError> {
        let (test_plan, sources) = RepTestPlan::try_load_and_resolve_from_github(
            &self.org,
            &self.repo,
            &self.path,
            self.git_ref,
            &ctx,
        )
        .await?;

        let variables = ParsedVariables::from_flat(self.variables.unwrap_or_default());

        PreparedPayload::prepare(test_plan, sources, variables, None, ctx).await
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PreparedPayload {
    pub test_plan: RepTestPlan,
    pub relative_files: SourceKeyedArrayMap<String>,
    pub custom_providers: SourceKeyedArrayMap<CustomProviderDefinition>,
    #[serde(default)]
    pub variables: Option<HashMap<String, ScalarOrArray>>,
}

impl PreparedPayload {
    pub async fn prepare(
        mut test_plan: RepTestPlan,
        sources: Sources,
        variables: ParsedVariables,
        vars_file_src: Option<SourceDir>,
        mut ctx: impl ResolutionContext,
    ) -> Result<PreparedPayload, PrepareError> {
        let flat = variables.as_flat();
        let variable_sources = variables.merge_into(&mut test_plan)?;
        ctx.set_sources(sources.with_variables_file(vars_file_src));
        let variables = (!flat.is_empty()).then_some(flat);

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

        let mut payload = Self {
            test_plan,
            relative_files: SourceKeyedArrayMap::from_data(files),
            custom_providers: SourceKeyedArrayMap::empty(),
            variables,
        };
        payload.set_custom_provider_definitions(Arc::unwrap_or_clone(
            ctx.custom_provider_definitions(),
        ));

        Ok(payload)
    }

    pub fn set_custom_provider_definitions(&mut self, custom_providers: CustomProviderDefinitions) {
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

        self.custom_providers = SourceKeyedArrayMap::from_data(raw_cps);
    }
}

async fn try_extract_relative_files(
    test_plan: &RepTestPlan,
    files: &mut HashMap<(StableSource, String), String>,
    ctx: &impl ResolutionContext,
) -> providers::Result<()> {
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

#[derive(Debug, thiserror::Error)]
pub enum PrepareError {
    #[error(transparent)]
    Format(#[from] formats::Error),

    #[error(transparent)]
    Providers(#[from] providers::Error),

    #[error(transparent)]
    Templating(#[from] templating::Errors),

    #[error(transparent)]
    Variables(#[from] variables::Error),
}
