use crate::{
    context::ResolutionContext,
    formats::{CustomProviderDeclaration, Error, Result},
    providers::{self, file::SourceDir},
    templating::CustomProviderDefinitions,
};
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// The source paths of each of the configs for a given test plan.
///
/// If the [ScenarioConfig] or [EnvironmentConfig] are specified inline then their source will
/// match that of the overall [TestPlanConfig], otherwise we store the source as defined in the
/// [RawTestPlanConfig].
#[derive(Debug, Default, Clone, PartialEq, Deserialize, Serialize)]
pub struct Sources {
    test_plan: SourceDir,
    scenario: Option<SourceDir>,
    environment: Option<SourceDir>,
    #[serde(default, skip)]
    custom_providers: Arc<CustomProviderDefinitions>,
}

impl Sources {
    pub(super) fn new(
        test_plan: SourceDir,
        scenario: Option<SourceDir>,
        environment: Option<SourceDir>,
    ) -> Self {
        Self {
            test_plan,
            scenario,
            environment,
            custom_providers: Default::default(),
        }
    }

    pub(super) async fn try_load_custom_providers(
        &mut self,
        tp: &[CustomProviderDeclaration],
        scenario: &[CustomProviderDeclaration],
        environment: &[CustomProviderDeclaration],
        ctx: &impl ResolutionContext,
    ) -> Result<()> {
        let mut errs = Vec::new();
        let mut custom_providers = CustomProviderDefinitions::default();

        let format_errors = |section: &str, errors: Vec<(String, providers::Error)>| {
            format!(
                "{section}:\n{}",
                errors
                    .into_iter()
                    .map(|(provider_name, e)| format!(" - {provider_name}: {e}"))
                    .join("\n")
            )
        };

        for declaration in tp.iter() {
            match declaration.try_load_all(self.test_plan(), ctx).await {
                Ok(providers) => {
                    for (name, (src, def)) in providers {
                        custom_providers.test_plan.insert(name.clone(), def);
                        custom_providers.sources.insert(name, src);
                    }
                }
                Err(errors) => errs.push(format_errors("test plan", errors)),
            }
        }

        for declaration in scenario.iter() {
            match declaration.try_load_all(self.scenario(), ctx).await {
                Ok(providers) => {
                    for (name, (src, def)) in providers {
                        custom_providers.scenario.insert(name.clone(), def);
                        custom_providers.sources.insert(name, src);
                    }
                }
                Err(errors) => errs.push(format_errors("scenario", errors)),
            }
        }

        for declaration in environment.iter() {
            match declaration.try_load_all(self.environment(), ctx).await {
                Ok(providers) => {
                    for (name, (src, def)) in providers {
                        custom_providers.environment.insert(name.clone(), def);
                        custom_providers.sources.insert(name, src);
                    }
                }
                Err(errors) => errs.push(format_errors("environment", errors)),
            }
        }

        if !errs.is_empty() {
            return Err(Error::FailedCustomProviderDefinitions { errs });
        }

        self.custom_providers = Arc::new(custom_providers);

        Ok(())
    }

    pub fn custom_provider_source(&self, name: &str) -> Option<&SourceDir> {
        self.custom_providers.source(name)
    }

    pub fn test_plan(&self) -> &SourceDir {
        &self.test_plan
    }

    /// The [Source] of the [EnvironmentConfig] in this test plan.
    ///
    /// Defaults to the source of the test plan itself if the environment was specified inline.
    pub fn environment(&self) -> &SourceDir {
        match self.environment.as_ref() {
            Some(source) => source,
            None => &self.test_plan,
        }
    }

    /// The [Source] of the [ScenarioConfig] in this test plan.
    ///
    /// Defaults to the source of the test plan itself if the scenario was specified inline.
    pub fn scenario(&self) -> &SourceDir {
        match self.scenario.as_ref() {
            Some(source) => source,
            None => &self.test_plan,
        }
    }

    pub fn custom_providers(&self) -> Arc<CustomProviderDefinitions> {
        self.custom_providers.clone()
    }
}

#[cfg(test)]
impl Sources {
    /// Test constructor that allows setting all fields including custom_providers
    pub fn with_custom_providers(
        test_plan: SourceDir,
        scenario: Option<SourceDir>,
        environment: Option<SourceDir>,
        custom_providers: Arc<CustomProviderDefinitions>,
    ) -> Self {
        Self {
            test_plan,
            scenario,
            environment,
            custom_providers,
        }
    }
}
