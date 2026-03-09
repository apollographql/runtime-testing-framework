use crate::{
    context::ResolutionContext,
    formats::{CustomProviderDeclaration, CustomProviderDefinition, Error, Result},
    providers::file::{CustomProviderSection, SourceDir, StableSource},
    templating::CustomProviderDefinitions,
};
use itertools::Itertools;
use std::{collections::HashMap, sync::Arc};

/// The source paths of each of the configs for a given test plan.
///
/// If the `ScenarioConfig` or `EnvironmentConfig` are specified inline then their source will
/// match that of the overall `TestPlanConfig`, otherwise we store the source as defined in the
/// `RawTestPlanConfig`.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Sources {
    test_plan: SourceDir,
    scenario: Option<SourceDir>,
    environment: Option<SourceDir>,
    custom_providers: Arc<CustomProviderDefinitions>,
    custom_provider_sources: HashMap<StableSource, SourceDir>,
    cli: SourceDir,
    variables_file: Option<SourceDir>,
}

impl Sources {
    pub fn new(
        test_plan: SourceDir,
        scenario: Option<SourceDir>,
        environment: Option<SourceDir>,
    ) -> Self {
        Self {
            test_plan,
            scenario,
            environment,
            custom_providers: Default::default(),
            custom_provider_sources: HashMap::new(),
            cli: SourceDir::default(),
            variables_file: None,
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
        let mut custom_provider_sources = HashMap::new();

        load_custom_providers(
            tp,
            self.test_plan(),
            CustomProviderSection::TestPlan,
            &mut custom_providers.test_plan,
            &mut custom_provider_sources,
            &mut errs,
            ctx,
        )
        .await;

        load_custom_providers(
            scenario,
            self.scenario(),
            CustomProviderSection::Scenario,
            &mut custom_providers.scenario,
            &mut custom_provider_sources,
            &mut errs,
            ctx,
        )
        .await;

        load_custom_providers(
            environment,
            self.environment(),
            CustomProviderSection::Environment,
            &mut custom_providers.environment,
            &mut custom_provider_sources,
            &mut errs,
            ctx,
        )
        .await;

        if !errs.is_empty() {
            return Err(Error::FailedCustomProviderDefinitions { errs });
        }

        self.custom_providers = Arc::new(custom_providers);
        self.custom_provider_sources = custom_provider_sources;

        Ok(())
    }

    /// Set the [SourceDir] for the standalone environment config being operated on.
    pub fn with_environment(mut self, source: SourceDir) -> Self {
        self.environment = Some(source);
        self
    }

    /// Set the [SourceDir] for the standalone scenario config being operated on.
    pub fn with_scenario(mut self, source: SourceDir) -> Self {
        self.scenario = Some(source);
        self
    }

    /// Register a single custom provider source for use in standalone `custom-provider` commands
    /// that operate on a definition file outside of a test plan.
    ///
    /// The provider is registered under [CustomProviderSection::TestPlan] so that path fields
    /// stamped with [StableSource::CustomProvider] during templating resolve correctly.
    pub fn with_custom_provider_source(mut self, ident: String, source: SourceDir) -> Self {
        self.custom_provider_sources.insert(
            StableSource::CustomProvider {
                section: CustomProviderSection::TestPlan,
                ident,
            },
            source,
        );
        self
    }

    /// Set the [SourceDir] for variables coming from the CLI (`--var k=v`).
    pub fn with_cli(mut self, source: SourceDir) -> Self {
        self.cli = source;
        self
    }

    /// Set the [SourceDir] for variables coming from a `--vars` file.
    ///
    /// Accepts `Option` so callers can forward the result of `Variables::merge` directly
    /// without an extra `if let`.
    pub fn with_variables_file(mut self, source: Option<SourceDir>) -> Self {
        self.variables_file = source;
        self
    }

    pub fn test_plan(&self) -> &SourceDir {
        &self.test_plan
    }

    pub fn cli(&self) -> &SourceDir {
        &self.cli
    }

    /// Returns the [SourceDir] for the variables file, or `None` if no `--vars` file was provided.
    pub fn variables_file(&self) -> Option<&SourceDir> {
        self.variables_file.as_ref()
    }

    /// The `SourceDir` of the `EnvironmentConfig` in this test plan.
    ///
    /// Defaults to the source of the test plan itself if the environment was specified inline.
    pub fn environment(&self) -> &SourceDir {
        match self.environment.as_ref() {
            Some(source) => source,
            None => &self.test_plan,
        }
    }

    /// The `SourceDir` of the `ScenarioConfig` in this test plan.
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

    pub(crate) fn source_dir_for(&self, src: &StableSource) -> &SourceDir {
        match src {
            StableSource::TestPlan => self.test_plan(),
            StableSource::Environment => self.environment(),
            StableSource::Scenario => self.scenario(),
            StableSource::CustomProvider { .. } => self
                .custom_provider_sources
                .get(src)
                .expect("custom provider source not found"),
            StableSource::Cli => self.cli(),
            StableSource::VariablesFile => self
                .variables_file
                .as_ref()
                .expect("VariablesFile source not set"),
        }
    }

    #[cfg(test)]
    /// Test constructor that allows setting all fields including custom_providers
    pub fn with_custom_providers(
        test_plan: SourceDir,
        scenario: Option<SourceDir>,
        environment: Option<SourceDir>,
        custom_providers: Arc<CustomProviderDefinitions>,
        custom_provider_sources: HashMap<StableSource, SourceDir>,
    ) -> Self {
        Self {
            test_plan,
            scenario,
            environment,
            custom_providers,
            custom_provider_sources,
            cli: SourceDir::default(),
            variables_file: None,
        }
    }
}

async fn load_custom_providers(
    declarations: &[CustomProviderDeclaration],
    source_dir: &SourceDir,
    section: CustomProviderSection,
    definitions: &mut HashMap<String, CustomProviderDefinition>,
    custom_provider_sources: &mut HashMap<StableSource, SourceDir>,
    errs: &mut Vec<String>,
    ctx: &impl ResolutionContext,
) {
    for declaration in declarations.iter() {
        let section_label = section.as_label();

        match declaration.try_load_all(source_dir, ctx).await {
            Ok(providers) => {
                for (ident, (source_dir, definition)) in providers {
                    match definitions.insert(ident.clone(), definition) {
                        Some(_) => errs.push(format!(
                            "{section_label}:\n - {ident}: duplicate custom provider name"
                        )),
                        None => {
                            custom_provider_sources.insert(
                                StableSource::CustomProvider { section, ident },
                                source_dir,
                            );
                        }
                    }
                }
            }
            Err(errors) => errs.push(format!(
                "{section_label}:\n{}",
                errors
                    .into_iter()
                    .map(|(provider_name, e)| format!(" - {provider_name}: {e}"))
                    .join("\n")
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        context::Context, formats::providers::test_helpers::create_temp_dir_with_file,
        providers::file::RawSource,
    };
    use indoc::indoc;

    const MINIMAL_PROVIDER: &str = indoc!(
        r#"
        name: my_provider
        description: A test provider
        command:
          name: run.sh
          kind: relative_path
          path: run.sh
        "#
    );

    fn declaration_with_provider(
        relative_path: &str,
        provider_name: &str,
    ) -> CustomProviderDeclaration {
        CustomProviderDeclaration {
            source: RawSource::Local {
                relative_path: relative_path.into(),
            },
            using: [(provider_name.to_string(), "provider.yaml".to_string())]
                .into_iter()
                .collect(),
        }
    }

    #[tokio::test]
    async fn duplicate_provider_name_within_section_is_an_error() {
        let (temp, _) = create_temp_dir_with_file("config.yaml", "");

        for dir in ["providers", "providers2"] {
            let providers_dir = temp.path().join(dir);
            std::fs::create_dir_all(&providers_dir).unwrap();
            std::fs::write(providers_dir.join("provider.yaml"), MINIMAL_PROVIDER).unwrap();
        }

        let tp_source = SourceDir::local(temp.path().canonicalize().unwrap());
        let mut sources = Sources::new(tp_source, None, None);

        let declarations = vec![
            declaration_with_provider("providers", "my_custom_provider"),
            declaration_with_provider("providers2", "my_custom_provider"),
        ];

        let result = sources
            .try_load_custom_providers(&declarations, &[], &[], &Context::new())
            .await;

        assert!(
            result.is_err(),
            "expected error for duplicate provider name"
        );
        let err = result.unwrap_err();
        let err_str = format!("{err:?}");
        assert!(
            err_str.contains("duplicate custom provider name"),
            "expected duplicate error, got: {err_str}"
        );
    }

    #[tokio::test]
    async fn same_provider_name_in_different_sections_is_not_an_error() {
        let (temp, _) = create_temp_dir_with_file("config.yaml", "");

        for dir in ["tp_providers", "scenario_providers"] {
            let providers_dir = temp.path().join(dir);
            std::fs::create_dir_all(&providers_dir).unwrap();
            std::fs::write(providers_dir.join("provider.yaml"), MINIMAL_PROVIDER).unwrap();
        }

        let tp_source = SourceDir::local(temp.path().canonicalize().unwrap());
        let mut sources = Sources::new(tp_source, None, None);

        let tp_declarations = vec![declaration_with_provider(
            "tp_providers",
            "my_custom_provider",
        )];
        let scenario_declarations = vec![declaration_with_provider(
            "scenario_providers",
            "my_custom_provider",
        )];

        let result = sources
            .try_load_custom_providers(
                &tp_declarations,
                &scenario_declarations,
                &[],
                &Context::new(),
            )
            .await;

        assert!(
            result.is_ok(),
            "expected no error for same name in different sections, got: {result:?}"
        );
    }
}
