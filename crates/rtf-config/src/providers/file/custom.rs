use crate::{
    SourceDir,
    checks::{self, Check},
    context::ResolutionContext,
    formats::CustomProviderDefinition,
    providers::{
        self,
        file::{AsUtf8FileContent, utility::FromCommand},
    },
    templating::{self, ErrorKind, Errors, Field, Result, Scalar, Template, TemplateContext},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// # Custom Provider
///
/// Use a custom provider to execute a command and produce a set of files.
///
/// ```yaml
/// - name: "router-docker-compose"
///   env_var: ROUTER_DOCKER_COMPOSE
///   kind: custom_provider
///   type: "router-docker-compose"
///   graph_ref: "graph@variant"
///   router_version: "v2.x.y"
///   build_router_from_source: "false"
/// ```
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct CustomProvider {
    /// The type of custom provider to use. This is the name of the custom provider to use.
    #[serde(rename = "type")]
    pub(crate) ty: String,

    /// The arguments to pass to the custom provider.
    #[serde(flatten)]
    pub(crate) arguments: HashMap<String, Field<Scalar>>,

    /// Set during TestPlan parsing as part of overrides and templating.
    #[serde(default, skip_serializing)]
    #[schemars(skip)]
    #[doc(hidden)]
    pub(crate) src: Option<SourceDir>,
}

impl CustomProvider {
    /// Expand [CustomProvider] into a fully templated [FromCommand]
    pub fn expand_and_template(
        &mut self,
        path: &mut Vec<String>,
        file_source: &SourceDir,
        file_ctx: &TemplateContext,
    ) -> Result<FromCommand> {
        let (definition_src, mut definition) = match file_ctx.custom_provider_definition(&self.ty) {
            Some((src, def)) => (src, def.clone()),
            None => {
                return Err(Errors::new(
                    ErrorKind::MissingCustomProvider,
                    &self.ty,
                    path,
                ));
            }
        };

        let definition_ctx = self.build_templating_context(
            path,
            file_source,
            file_ctx,
            &definition,
            definition_src,
        )?;

        path.push("definition".into());
        definition.try_template(path, definition_src, &definition_ctx)?;

        Ok(FromCommand::new(definition.command))
    }

    fn build_templating_context(
        &mut self,
        path: &[String],
        file_src: &SourceDir,
        file_ctx: &TemplateContext,
        definition: &CustomProviderDefinition,
        definition_src: &SourceDir,
    ) -> Result<TemplateContext> {
        // First we template the arguments of this `CustomProvider` using the
        // source and context for the file containing it. As we template arguments
        // we build a new map of the resolved arguments to use when templating
        // the `CustomProviderDefinition`.
        let mut errs = templating::ErrorBuilder::new();
        let mut argument_sources = HashMap::with_capacity(self.arguments.len());
        for (k, argument) in self.arguments.iter_mut() {
            let mut argument_path = path.to_owned();
            argument_path.push(k.to_string());

            // Next we match on Pending/Resolved and create a map of sources for each argument
            // Pending fields take their source from the file context variables
            // Resolved fields take their source from the file the `CustomProvider` is defined
            // in or the `CustomProvider` source if one is provided.
            match argument {
                Field::Pending(variable) => {
                    // argument.try_template will error on the missing variable below so None is not used
                    if let Some((var_src, _)) = file_ctx.get_with_source(variable) {
                        argument_sources.insert(k.clone(), var_src.clone());
                    }
                }
                Field::Resolved(_) => {
                    argument_sources
                        .insert(k.clone(), self.src.as_ref().unwrap_or(file_src).clone());
                }
            }

            errs.append(argument.try_template(&mut argument_path, file_src, file_ctx));
        }
        errs.into_result(())?;

        let resolved_arguments: HashMap<String, Scalar> = self
            .arguments
            .drain()
            .map(|(k, argument)| (k, argument.into_resolved()))
            .collect();

        // Next, we build the context needed for templating the definition file
        // and we extend that with the resolved arguments from above. The extend method
        // will preserve the source of each variable.
        let mut definition_ctx =
            file_ctx.for_config_file(definition_src, None, definition.variable_definitions.iter());
        definition_ctx.extend_with_sources(argument_sources, resolved_arguments);

        Ok(definition_ctx)
    }
}

impl Template for CustomProvider {
    fn has_pending_fields(&self) -> bool {
        self.arguments.values().any(|v| v.has_pending_fields())
    }

    fn required_variables(&self) -> Vec<String> {
        self.arguments
            .values()
            .flat_map(|v| v.required_variables())
            .collect()
    }

    fn validate_context(
        &self,
        path: &mut Vec<String>,
        allowed_variables: &HashSet<&String>,
        file_source: &SourceDir,
        ctx: &TemplateContext,
    ) -> templating::Result<()> {
        let mut errs = templating::ErrorBuilder::new();
        errs.append(
            self.arguments
                .validate_context(path, allowed_variables, file_source, ctx),
        );

        match ctx.custom_provider_definition(&self.ty) {
            Some((def_src, def)) => {
                // We don't reuse `build_templating_context` here as that actually resolves the
                // fields in our `arguments` map and cares about the value associated with each
                // variable. Here, all we care about is the fact that the correct variables are
                // defined.
                let ctx = ctx.for_config_file(def_src, None, def.variable_definitions.iter());
                let allowed_variables: HashSet<&String> = self
                    .arguments
                    .keys()
                    .chain(
                        def.variable_definitions
                            .iter()
                            .filter(|vd| vd.default.is_some())
                            .map(|vd| &vd.name),
                    )
                    .collect();

                errs.append(def.validate_context(path, &allowed_variables, def_src, &ctx));
            }

            None => errs.push(ErrorKind::MissingCustomProvider, &self.ty, path),
        };

        errs.into_result(())
    }

    fn try_template(
        &mut self,
        path: &mut Vec<String>,
        file_source: &SourceDir,
        ctx: &TemplateContext,
    ) -> templating::Result<()> {
        self.arguments.try_template(path, file_source, ctx)
    }
}

// AsUtf8FileContent needs to be implemented for CustomProvider to be a valid FileProvider that can be added to the NamedFileProvider enum
// However, the AsUtf8FileContent methods should never actually be called
impl AsUtf8FileContent for CustomProvider {
    async fn try_get_file_content(
        &self,
        _ctx: &impl ResolutionContext,
    ) -> providers::Result<String> {
        panic!(
            "Should not be able to get here. Custom provider should have been expanded when templating the config."
        )
    }
}

// Check needs to be implemented for CustomProvider to be a valid FileProvider that can be added to the NamedFileProvider enum
// However, the Check methods should never actually be called
impl Check for CustomProvider {
    fn try_check(
        &self,
        _path: &mut Vec<String>,
        _ctx: &impl ResolutionContext,
    ) -> checks::Result<()> {
        panic!(
            "Should not be able to get here. Custom provider should have been expanded when templating the config."
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        VariableDefinition,
        context::Context,
        providers::{
            command::CommandSection,
            file::{FileProvider, NamedFileProvider, RelativeFile},
        },
        templating::{CustomProviderDefinitions, ErrorKind},
    };
    use simple_test_case::test_case;
    use std::sync::Arc;

    // A helper function for testing the `build_templating_context` method
    fn test_build_templating_context(
        def_var_default: Option<Scalar>,
        def_field: Field<String>,
        provider_argument: Option<(&str, Field<Scalar>)>,
        provider_src: Option<&str>,
        file_ctx: Option<(&str, &str, SourceDir)>,
    ) -> Result<TemplateContext> {
        // There are two parts of the `CustomProviderDefinition`` we want to parameterise
        // 1. The default value for the definition variable (Some or None)
        // 2. The path field for relative.txt (Pending or Resolved)
        let definition = CustomProviderDefinition {
            variable_definitions: vec![VariableDefinition {
                name: "definition_variable".to_string(),
                description: "".to_string(),
                default: def_var_default,
            }],
            command: CommandSection {
                file_providers: vec![NamedFileProvider {
                    name: "relative.txt".to_string(),
                    env_var: "RELATIVE".to_string(),
                    provider: FileProvider::RelativePath(RelativeFile {
                        path: def_field,
                        // The source for the relative file is not used in these tests
                        // We test setting correct sources for relative files elsewhere
                        src: None,
                    }),
                }],
                ..CommandSection::empty()
            },
            ..CustomProviderDefinition::empty()
        };

        // The arguments for the custom provider can be specified or in the case where
        // we want to test definition defaults are used, can be left empty
        let mut arguments: HashMap<String, Field<Scalar>> = HashMap::new();

        if let Some((argument_name, argument)) = provider_argument {
            arguments.insert(argument_name.to_string(), argument);
        }

        // The provider uses the arguments defined above
        // The source can be one of two values:
        // 1. No source means the provider has not been overridden from the test plan
        // 2. A source means the provider has been overridden from the test plan and
        //    the source is therefore test plan's source
        let mut provider = CustomProvider {
            ty: "custom-provider".to_string(),
            arguments,
            src: provider_src.map(SourceDir::local),
        };

        // The file context can either contain no variables or sources or,
        // for the cases we want to test variables coming from sources external
        // to the provider or definition, can set variables and their sources
        let mut variables = HashMap::new();
        let mut override_sources = HashMap::new();
        if let Some((var_name, var_value, src)) = file_ctx {
            variables.insert(var_name.to_string(), var_value.into());
            override_sources.insert(var_name.to_string(), src);
        }
        let file_ctx = TemplateContext::new(
            variables,
            SourceDir::local("/test-plan"),
            override_sources,
            Default::default(),
        );

        provider.build_templating_context(
            &[],                            // The path is empty as this is not used in the test assertions
            &SourceDir::local("/provider"), // This is the source for the file the custom provider is defined in
            &file_ctx,
            &definition,
            &SourceDir::local("/definition"), // This is the source for the file the custom provider definition is in
        )
    }

    #[test_case(None; "provider not from overrides")]
    #[test_case(Some("/test-plan"); "provider from overrides")]
    #[test]
    /// This is the simplest test case for `build_templating_context`. The custom provider definition has no fields that need templating,
    /// so no variables come back from the context
    ///
    /// [FileContext(No variable values)] -> [CustomProvider(No arguments)] -> [CustomProviderDefinition(No defaults)] -> [Resolved(FileProvider)]
    fn custom_provider_build_templating_context_resolved_field_in_definition_success(
        provider_src: Option<&str>,
    ) {
        let res = test_build_templating_context(
            None,
            Field::Resolved("from definition".into()),
            None,
            provider_src,
            None,
        );
        assert!(res.is_ok(), "expected TemplateContext, got {res:?}");

        let ctx = res.unwrap();
        assert_eq!(ctx.variables(), &HashMap::new());
    }

    #[test_case(None; "provider not from overrides")]
    #[test_case(Some("/test-plan"); "provider from overrides")]
    /// This is counter-intuitive and included for completeness
    /// This is not a valid state but is not an error condition we catch in this function. It should be caught in pre-templating checks and,
    /// if not, should error in `expand_and_template`.
    /// This results in an empty variables map since no variables can be successfully templated.
    ///
    /// [FileContext(No variable values)] -> [CustomProvider(No arguments)] -> [CustomProviderDefinition(No defaults)] -> [Pending(FileProvider)]
    #[test]
    fn custom_provider_build_templating_context_pending_field_in_definition_missing_variable_definition_success(
        provider_src: Option<&str>,
    ) {
        let res = test_build_templating_context(
            None,
            Field::Pending("undefined_variable".into()),
            None,
            provider_src,
            None,
        );
        assert!(res.is_ok(), "expected TemplateContext, got {res:?}");

        let ctx = res.unwrap();
        assert_eq!(ctx.variables(), &HashMap::new());
    }

    #[test_case(None; "provider not from overrides")]
    #[test_case(Some("/test-plan"); "provider from overrides")]
    #[test]
    /// This is counter-intuitive and included for completeness
    /// This is not a valid state but is not an error condition we catch in this function. It should be caught in pre-templating checks and,
    /// if not, should error in `expand_and_template`.
    /// This results in an empty variables map since no variables can be successfully templated.
    ///
    /// [FileContext(No variable values)] -> [CustomProvider(No arguments)] -> [CustomProviderDefinition(No defaults)] -> [Pending(FileProvider)]
    fn custom_provider_build_templating_context_pending_field_in_definition_no_default_value_no_resolved_arg_no_file_ctx_variable_success(
        provider_src: Option<&str>,
    ) {
        let res = test_build_templating_context(
            None,
            Field::Pending("definition_variable".into()),
            None,
            provider_src,
            None,
        );
        assert!(res.is_ok(), "expected TemplateContext, got {res:?}");

        let ctx = res.unwrap();
        assert_eq!(ctx.variables(), &HashMap::new());
    }

    #[test_case(None; "provider not from overrides")]
    #[test_case(Some("/test-plan"); "provider from overrides")]
    #[test]
    /// This is the next simplest success case, there are no values provided in either the provider arguments or file variables.
    /// The value used is from the provider definition's default.
    ///
    /// [FileContext(No variable values)] -> [CustomProvider(No arguments)] -> [CustomProviderDefinition(Default value)] -> [Pending(FileProvider)]
    fn custom_provider_build_templating_context_pending_field_in_definition_uses_default_success(
        provider_src: Option<&str>,
    ) {
        let res = test_build_templating_context(
            Some("definition default value".into()),
            Field::Pending("definition_variable".into()),
            None,
            provider_src,
            None,
        );
        assert!(res.is_ok(), "expected TemplateContext, got {res:?}");

        let ctx = res.unwrap();
        assert_eq!(
            ctx.get_with_source("definition_variable"),
            Some((
                &SourceDir::local("/definition"),
                &"definition default value".into()
            ))
        );
    }

    #[test_case(None, SourceDir::local("/provider"); "provider not from overrides")]
    #[test_case(Some("/test-plan"), SourceDir::local("/test-plan"); "provider from overrides")]
    #[test]
    /// This tests a variable's value coming from the provider arguments. A default is included to show that the argument value takes precedence.
    /// This is the situation where the custom provider's source is relevant.
    /// If the provider did not come from overrides then the source is the file itself.
    /// If the provider came from the test plan overrides then the source must be the test plan itself (which is set during the test plan parsing)
    ///
    /// [FileContext(No variable values)] -> [CustomProvider(Resolved(argument))] -> [CustomProviderDefinition(Default value)] -> [Pending(FileProvider)]
    fn custom_provider_build_templating_context_pending_field_in_definition_uses_resolved_provider_argument_success(
        provider_src: Option<&str>,
        expected_src: SourceDir,
    ) {
        let res = test_build_templating_context(
            Some("definition_default_value".into()), // We still supply a default to sanity check the value from provider arguments overrides it
            Field::Pending("definition_variable".into()),
            Some((
                "definition_variable",
                Field::Resolved("resolved value from provider".into()),
            )),
            provider_src,
            None,
        );
        assert!(res.is_ok(), "expected TemplateContext, got {res:?}");

        let ctx = res.unwrap();
        assert_eq!(
            ctx.get_with_source("definition_variable"),
            Some((&expected_src, &"resolved value from provider".into()))
        );
    }

    #[test_case(None; "provider not from overrides")]
    #[test_case(Some("/test-plan"); "provider from overrides")]
    /// This tests a variable's value coming from the file context's variables.
    /// It checks that the variable's source is from the file context and superceeds any defaults
    ///
    /// [FileContext(Variable values set)] -> [CustomProvider(Pending(argument))] -> [CustomProviderDefinition(Default value)] -> [Pending(FileProvider)]
    #[test]
    fn custom_provider_build_templating_context_pending_field_in_provider_uses_file_context_variable_success(
        provider_src: Option<&str>,
    ) {
        let res = test_build_templating_context(
            Some("definition_default_value".into()), // We still supply a default to sanity check the value from provider fields overrides it,
            Field::Pending("definition_variable".into()),
            Some((
                "definition_variable",
                Field::Pending("provider_variable".into()),
            )),
            provider_src,
            Some((
                "provider_variable",
                "value from file context",
                SourceDir::local("wherever this variable came from"),
            )),
        );
        assert!(res.is_ok(), "expected TemplateContext, got {res:?}");

        let ctx = res.unwrap();
        assert_eq!(
            ctx.get_with_source("definition_variable"),
            Some((
                &SourceDir::local("wherever this variable came from"),
                &"value from file context".into()
            ))
        );
    }

    #[test_case(None; "provider not from overrides")]
    #[test_case(Some("/test-plan"); "provider from overrides")]
    #[test]
    /// This tests an error path, the custom provider argument is pending, but no variable values are set in the
    /// file context. This will fail with an UnknownVariable error.
    ///
    /// [FileContext(No variable values)] -> [CustomProvider(Pending(argument))] -> [CustomProviderDefinition(Default value)] -> [Pending(FileProvider)]
    fn custom_provider_build_templating_context_pending_field_in_provider_missing_value_error(
        provider_src: Option<&str>,
    ) {
        let res = test_build_templating_context(
            None,
            Field::Pending("definition_variable".into()),
            Some((
                "definition_variable",
                Field::Pending("variable_not_defined".into()),
            )),
            provider_src,
            None,
        );
        assert!(res.is_err(), "expected error, got {res:?}");

        let err = res.unwrap_err().unwrap_single();
        assert_eq!(err.kind, ErrorKind::UnknownVariable);
        assert_eq!(err.message, "variable_not_defined".to_string());
    }

    #[test]
    #[should_panic(
        expected = "Should not be able to get here. Custom provider should have been expanded when templating the config."
    )]
    fn custom_provider_check_is_missing() {
        let custom_provider = CustomProvider {
            ty: "my-custom-provider".to_string(),
            arguments: HashMap::new(),
            src: None,
        };

        let ctx = Context::new();

        let _res = custom_provider.try_check(&mut Vec::new(), &ctx);
    }

    #[tokio::test]
    #[should_panic(
        expected = "Should not be able to get here. Custom provider should have been expanded when templating the config."
    )]
    async fn custom_provider_try_get_file_content_panics() {
        let custom_provider = CustomProvider {
            ty: "my-custom-provider".to_string(),
            arguments: HashMap::new(),
            src: None,
        };

        let ctx = Context::new();

        let _res = custom_provider.try_get_file_content(&ctx).await;
    }

    #[test]
    fn custom_provider_validate_context_uses_variable_defaults() {
        let definition = CustomProviderDefinition {
            variable_definitions: vec![VariableDefinition {
                name: "var_with_default".to_string(),
                description: "".to_string(),
                // Attempting to validate a context without any variables using this definition
                // should succeed due to this default value
                default: Some(1.into()),
            }],
            command: CommandSection {
                file_providers: vec![NamedFileProvider {
                    name: "relative.txt".to_string(),
                    env_var: "RELATIVE".to_string(),
                    provider: FileProvider::RelativePath(RelativeFile {
                        path: Field::Pending("var_with_default".into()),
                        src: None,
                    }),
                }],
                ..CommandSection::empty()
            },
            ..CustomProviderDefinition::empty()
        };

        let custom_provider = CustomProvider {
            ty: "my-custom-provider".to_string(),
            arguments: HashMap::new(),
            src: None,
        };

        let res = custom_provider.validate_context(
            &mut Vec::new(),
            &HashSet::new(),
            &SourceDir::local("/"),
            &TemplateContext::new(
                HashMap::new(),
                SourceDir::local("/"),
                HashMap::new(),
                Arc::new(CustomProviderDefinitions {
                    test_plan: HashMap::from([(
                        "my-custom-provider".to_string(),
                        (SourceDir::local("/providers"), definition),
                    )]),
                    ..Default::default()
                }),
            ),
        );

        assert!(res.is_ok(), "expected validation to succeed, got {res:?}");
    }
}
