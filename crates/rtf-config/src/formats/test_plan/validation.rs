use crate::{
    StableSource, VariableDefinition,
    context::ResolutionContext,
    formats::TestPlanConfig,
    templating::{self, CustomProviderDefinitions, Scalar, Template, TemplateContext},
};
use std::collections::{HashMap, HashSet};

impl TestPlanConfig {
    /// The set of allowed templating variables that this test plan supports.
    ///
    /// This is the union of variables defined as a scalars and those that are part of a matrix
    pub(super) fn allowed_variables(&self) -> HashSet<&String> {
        self.variables.keys().chain(self.matrix.keys()).collect()
    }

    /// Checks whether templating will work for the current test plan configuration.
    ///
    /// This method performs a comprehensive validation of the test plan's templating variables and their values,
    /// ensuring that all variable definitions are valid, all variable values conform to their allowed values,
    /// and that there are no conflicting or incompatible variable definitions. It also validates the matrix
    /// configuration for conflicting keys and dimensions, and checks the overall template context for errors.
    pub fn check_templating_will_work(
        &mut self,
        variable_sources: &HashMap<String, StableSource>,
        ctx: &impl ResolutionContext,
    ) -> templating::Result<()> {
        let tp_source = StableSource::TestPlan;
        let custom_providers = ctx.custom_provider_definitions();
        let stub_variables = self
            .allowed_variables()
            .into_iter()
            .map(|var| (var.to_string(), Scalar::String(var.to_string())))
            .collect();

        let template_ctx =
            TemplateContext::new(stub_variables, HashMap::new(), custom_providers.clone());

        let mut errs = templating::ErrorBuilder::new();
        self.validate_all_variable_definitions(&custom_providers, &mut errs);

        let effective_allowed = self.compute_effective_allowed_values(&custom_providers, &mut errs);
        self.validate_values_against_allowed(variable_sources, &effective_allowed, &mut errs);

        self.matrix
            .check_conflicting_keys(&self.variables, &mut errs);
        self.matrix.check_dimensions(&mut errs);
        errs.append(self.validate_context(
            &mut Vec::new(),
            &HashSet::new(), // overwritten in self.validate_context
            &tp_source,
            &template_ctx,
        ));

        errs.into_result(())
    }

    /// Returns all variable definition sources as (path, variable_definitions) pairs.
    fn variable_definition_sources(
        &self,
        custom_providers: &CustomProviderDefinitions,
    ) -> Vec<(Vec<String>, Vec<VariableDefinition>)> {
        let mut sources: Vec<(Vec<String>, Vec<VariableDefinition>)> = vec![
            (
                vec!["environment".into()],
                self.environment.variable_definitions.clone(),
            ),
            (
                vec!["scenario".into()],
                self.scenario.variable_definitions.clone(),
            ),
        ];

        let sections = [
            &custom_providers.test_plan,
            &custom_providers.scenario,
            &custom_providers.environment,
        ];
        for definitions in sections.into_iter() {
            for (name, def) in definitions.iter() {
                sources.push((
                    vec!["custom_providers".into(), name.clone()],
                    def.variable_definitions.clone(),
                ));
            }
        }

        sources
    }

    /// Validate all the variable definitions
    fn validate_all_variable_definitions(
        &self,
        custom_providers: &CustomProviderDefinitions,
        errs: &mut templating::ErrorBuilder,
    ) {
        for (path, variable_definitions) in self
            .variable_definition_sources(custom_providers)
            .into_iter()
        {
            for vd in variable_definitions.iter() {
                let full_path: Vec<String> = path
                    .iter()
                    .cloned()
                    .chain(["variable_definitions".into(), vd.name.clone()])
                    .collect();
                vd.validate(&full_path, errs);
            }
        }
    }

    /// Collect all variable definitions grouped by name, with their source paths for error messages
    fn collect_variable_definitions_by_name(
        &self,
        custom_providers: &CustomProviderDefinitions,
    ) -> HashMap<String, Vec<(Vec<String>, VariableDefinition)>> {
        let mut defs_by_name: HashMap<String, Vec<(Vec<String>, VariableDefinition)>> =
            HashMap::new();

        for (path, variable_definitions) in
            self.variable_definition_sources(custom_providers).iter()
        {
            for vd in variable_definitions.iter() {
                let full_path: Vec<String> = path
                    .iter()
                    .cloned()
                    .chain(["variable_definitions".into()])
                    .collect();
                defs_by_name
                    .entry(vd.name.clone())
                    .or_default()
                    .push((full_path, vd.clone()));
            }
        }

        defs_by_name
    }

    /// Compute the effective allowed values for each variable by intersecting all definitions.
    /// Returns an empty vec for a variable if it's unconstrained (no allowed_values defined).
    /// Adds errors to `errs` if definitions have incompatible (empty intersection) allowed values.
    fn compute_effective_allowed_values(
        &self,
        custom_providers: &CustomProviderDefinitions,
        errs: &mut templating::ErrorBuilder,
    ) -> HashMap<String, Vec<Scalar>> {
        let definitions_by_name = self.collect_variable_definitions_by_name(custom_providers);

        let mut effective: HashMap<String, Vec<Scalar>> = HashMap::new();

        for (var_name, definitions) in definitions_by_name.into_iter() {
            // Collect all Some(allowed_values) from definitions, skipping empty arrays
            // (empty arrays are already reported as EmptyAllowedValues errors)
            let constrained: Vec<_> = definitions
                .iter()
                .filter_map(|(path, vd)| {
                    vd.allowed_values
                        .as_ref()
                        .filter(|av| !av.is_empty())
                        .map(|av| (path, av))
                })
                .collect();

            if constrained.is_empty() {
                // All definitions have allowed_values: None -> unconstrained
                continue;
            }

            // Start with first constrained set, intersect with rest
            let mut intersection: Vec<Scalar> = constrained[0].1.clone();

            for (_, allowed) in constrained.iter().skip(1) {
                intersection.retain(|v| allowed.contains(v));
            }

            if intersection.is_empty() {
                // Build error message showing conflicting definitions
                let locations: Vec<_> = constrained
                    .iter()
                    .map(|(path, av)| format!("  - {}: {:?}", path.join("."), av))
                    .collect();

                errs.push(
                    templating::ErrorKind::IncompatibleAllowedValues,
                    format!(
                        "variable '{}' has incompatible allowed_values (no common values):\n{}",
                        var_name,
                        locations.join("\n")
                    ),
                    std::slice::from_ref(&var_name),
                );
            } else {
                effective.insert(var_name.clone(), intersection);
            }
        }

        effective
    }

    /// Validate that all variable values are in their effective allowed values.
    fn validate_values_against_allowed(
        &self,
        variable_sources: &HashMap<String, StableSource>,
        effective_allowed: &HashMap<String, Vec<Scalar>>,
        errs: &mut templating::ErrorBuilder,
    ) {
        let mut check = |var_name: &str, value: &Scalar, source: &str, path: Vec<String>| {
            if let Some(allowed) = effective_allowed.get(var_name)
                && !allowed.contains(value)
            {
                errs.push(
                    templating::ErrorKind::ValueNotAllowed,
                    format!(
                        "{} '{}' has value '{}' not in allowed: {:?}",
                        source, var_name, value, allowed
                    ),
                    &path,
                );
            }
        };

        // Check self.variables (includes CLI variables merged in)
        for (name, value) in self.variables.iter() {
            let source = if variable_sources.contains_key(name) {
                "CLI variable"
            } else {
                "test plan variable"
            };
            check(name, value, source, vec!["variables".into(), name.clone()]);
        }

        // Check matrix dimensions
        for (name, values) in self.matrix.dimensions.iter() {
            let source = if variable_sources.contains_key(name) {
                "CLI matrix dimension"
            } else {
                "matrix dimension"
            };
            for value in values {
                check(
                    name,
                    value,
                    source,
                    vec!["matrix".into(), "dimensions".into(), name.clone()],
                );
            }
        }

        // Check matrix include
        for (idx, include_map) in self.matrix.include.iter().enumerate() {
            for (name, value) in include_map.iter() {
                check(
                    name,
                    value,
                    &format!("matrix include[{}]", idx),
                    vec![
                        "matrix".into(),
                        "include".into(),
                        idx.to_string(),
                        name.clone(),
                    ],
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        SourceDir, StableSource, VariableDefinition,
        context::{Context, ResolutionContext},
        formats::{
            CustomProviderDeclaration, CustomProviderDefinition, EnvironmentConfig, Matrix,
            ScenarioConfig, TestPlanConfig,
            environment::{
                EnvironmentExecution, ScriptEnvironment, test_helpers::templatable_environment,
            },
            scenario::{ScenarioExecution, test_helpers::templatable_scenario},
            test_plan::Sources,
            tests::{named_file_provider_with_field, p, template_context},
        },
        providers::command::CommandSection,
        templating::{CustomProviderDefinitions, ErrorBuilder, ErrorKind, Scalar},
        variables_map,
    };
    use simple_test_case::test_case;
    use std::{collections::HashMap, sync::Arc};

    /// Create a TestPlanConfig for template tests
    fn templatable_test_plan(
        variables: HashMap<String, Scalar>,
        dimensions: HashMap<String, Vec<Scalar>>,
        include: Vec<HashMap<String, Scalar>>,
        scenario_fields: &[&str],
        setup_fields: &[&str],
        teardown_fields: &[&str],
        custom_providers: &[CustomProviderDeclaration],
    ) -> TestPlanConfig {
        let mut env_variables = setup_fields.to_vec();
        env_variables.extend_from_slice(teardown_fields);

        TestPlanConfig {
            variables,
            matrix: Matrix {
                variant_names: None,
                dimensions,
                include,
            },
            custom_providers: custom_providers.to_vec(),
            scenario: templatable_scenario(scenario_fields, scenario_fields, &[]),
            environment: templatable_environment(
                &env_variables,
                setup_fields,
                teardown_fields,
                &[],
            ),
            ..TestPlanConfig::empty()
        }
    }

    // Helper to create a TestPlanConfig with custom provider definitions for testing.
    // Returns (TestPlanConfig, Context) where the context holds the custom provider sources.
    fn test_plan_with_custom_provider_definitions(
        custom_provider_variable_definitions: Vec<VariableDefinition>,
    ) -> (TestPlanConfig, Context) {
        let custom_provider_def = CustomProviderDefinition {
            name: "test_provider".into(),
            description: "A test provider".into(),
            variable_definitions: custom_provider_variable_definitions,
            command: CommandSection::empty(),
        };

        let mut custom_providers = CustomProviderDefinitions::default();
        custom_providers
            .test_plan
            .insert("test_provider".into(), custom_provider_def);

        let sources = Sources::with_custom_providers(
            SourceDir::default(),
            None,
            None,
            Arc::new(custom_providers),
            Default::default(),
        );

        let mut ctx = Context::new();
        ctx.set_sources(sources);

        (TestPlanConfig::empty(), ctx)
    }

    /// Create a matrix with two variables per key provided
    fn dimensions_from_keys(keys: &[&str], no_entries: isize) -> HashMap<String, Vec<Scalar>> {
        keys.iter()
            .map(|&key| {
                (
                    key.to_string(),
                    if no_entries <= 0 {
                        Vec::new()
                    } else {
                        (0..no_entries)
                            .map(|i| format!("{key}{}", i + 1).into())
                            .collect()
                    },
                )
            })
            .collect()
    }

    /// Create a VariableDefinition with a default variable
    fn variable_with_default(name: &str, val: &str) -> VariableDefinition {
        VariableDefinition {
            name: name.into(),
            description: String::default(),
            default: Some(val.into()),
            allowed_values: None,
        }
    }

    /// Create a VariableDefinition with allowed_values
    fn variable_with_allowed_values(
        name: &str,
        default: Option<&str>,
        allowed_values: Option<Vec<&str>>,
    ) -> VariableDefinition {
        VariableDefinition {
            name: name.into(),
            description: String::default(),
            default: default.map(|v| v.into()),
            allowed_values: allowed_values.map(|vals| vals.into_iter().map(|v| v.into()).collect()),
        }
    }

    #[test_case(&["scenario", "setup", "teardown"], &[], &[]; "all in variables")]
    #[test_case(&[], &["scenario", "setup", "teardown"], &[]; "all in dimensions")]
    #[test_case(&[], &[], &["scenario", "setup", "teardown"]; "all in include")]
    #[test_case(&["scenario"], &["setup"], &["teardown"]; "one in each")]
    #[test]
    fn check_templating_will_work_success(
        variable_keys: &[&str],
        dimension_keys: &[&str],
        include_keys: &[&str],
    ) {
        let variables = template_context(variable_keys);
        let matrix = dimensions_from_keys(dimension_keys, 1);
        let include = vec![template_context(include_keys).variables().clone()];
        let mut test_plan = templatable_test_plan(
            variables.variables().clone(),
            matrix,
            include,
            &["scenario"],
            &["setup"],
            &["teardown"],
            &[],
        );

        let res = test_plan.check_templating_will_work(&HashMap::new(), &Context::new());
        assert!(
            res.is_ok(),
            "expected templating will work to succeed, got {:?}",
            res
        );
    }

    #[test_case(&["foo"], &[], "foo"; "single conflicting key dimensions and variables")]
    #[test_case(&["foo", "bar", "baz"], &[], "bar, baz, foo"; "multiple conflicting keys dimensions and variables")]
    #[test_case(&[], &["foo"], "foo"; "single conflicting key include and variables")]
    #[test_case(&[], &["foo", "bar", "baz"], "bar, baz, foo"; "multiple conflicting keys include and variables")]
    #[test_case(&["a"], &["a"], "a"; "single conflicting key dimensions and include")]
    #[test_case(&["a", "b", "c"], &["a", "b", "c"], "a, b, c"; "multiple conflicting keys dimensions and include")]
    #[test]
    fn check_templating_will_work_conflicting_keys_errors(
        dimension_keys: &[&str],
        include_keys: &[&str],
        expected_err_message: &str,
    ) {
        let variables = template_context(&["foo", "bar", "baz"]).variables().clone();
        let dimensions = dimensions_from_keys(dimension_keys, 2);
        let include = vec![template_context(include_keys).variables().clone()];
        let mut test_plan =
            templatable_test_plan(variables, dimensions, include, &[], &[], &[], &[]);

        let expected_err_kind = ErrorKind::ConflictingVariables;

        let res = test_plan.check_templating_will_work(&HashMap::new(), &Context::new());
        assert!(
            res.is_err(),
            "expected templating will work to fail, got {:?}",
            res
        );

        let error = res.unwrap_err().unwrap_single();
        assert_eq!(
            error.kind, expected_err_kind,
            "test that the error kind is as expected"
        );
        assert_eq!(
            error.message, expected_err_message,
            "test that error message is as expected"
        );
    }

    #[test_case(&["foo"], &["foo"]; "single matrix")]
    #[test_case(&["foo", "bar", "baz"], &["bar", "baz", "foo"]; "multiple matrices")]
    #[test]
    fn check_templating_will_work_empty_dimension_errors(
        dimension_keys: &[&str],
        expected_err_messages: &[&str],
    ) {
        let variables = template_context(&[]).variables().clone();
        let dimensions = dimensions_from_keys(dimension_keys, 0);
        let include = Vec::new();
        let mut test_plan =
            templatable_test_plan(variables, dimensions, include, &[], &[], &[], &[]);

        let expected_err_kind = ErrorKind::EmptyMatrixVariable;

        let res = test_plan.check_templating_will_work(&HashMap::new(), &Context::new());
        assert!(
            res.is_err(),
            "expected templating will work to fail, got {:?}",
            res
        );

        let errors = res.unwrap_err();
        assert_eq!(
            errors.iter().count(),
            expected_err_messages.len(),
            "test that the expected number of errors occur"
        );
        assert!(
            errors
                .iter()
                .all(|e| matches!(e.kind, ErrorKind::EmptyMatrixVariable)),
            "expected all errors to be {:?}, got {:?}",
            expected_err_kind,
            errors
        );
        let mut err_messages: Vec<String> = errors.iter().map(|f| f.message.clone()).collect();
        err_messages.sort();
        assert_eq!(
            err_messages, expected_err_messages,
            "test that the error messages are as expected"
        );
    }

    #[test]
    fn check_templating_will_work_inconsistent_dimension_variable_errors() {
        let variables = HashMap::new();
        let mut dimensions: HashMap<String, Vec<Scalar>> = HashMap::new();
        dimensions.insert("foo".into(), vec!["a".into(), 42.into()]);
        let mut test_plan =
            templatable_test_plan(variables, dimensions, vec![], &[], &[], &[], &[]);

        let expected_err_kind = ErrorKind::InconsistentMatrixVariable;
        let expected_err_message = "foo";

        let res = test_plan.check_templating_will_work(&HashMap::new(), &Context::new());
        assert!(
            res.is_err(),
            "expected templating will work to fail, got {:?}",
            res
        );

        let error = res.unwrap_err().unwrap_single();
        assert_eq!(
            error.kind, expected_err_kind,
            "test that the error kind is as expected"
        );
        assert_eq!(
            error.message, expected_err_message,
            "test that error message is as expected"
        );
    }

    #[test_case(vec![variables_map!("foo" => "a"), variables_map!("bar" => "b")]; "key names")]
    #[test_case(vec![variables_map!("foo" => "a"), variables_map!("foo" => 42)]; "variable types")]
    #[test]
    fn check_templating_will_work_inconsistent_include_errors(
        include: Vec<HashMap<String, Scalar>>,
    ) {
        let variables = HashMap::new();
        let dimensions = HashMap::new();
        let mut test_plan =
            templatable_test_plan(variables, dimensions, include, &[], &[], &[], &[]);

        let expected_err_kind = ErrorKind::InconsistentMatrixInclude;
        let expected_err_message = "matrix include maps must share consistent keys and types";

        let res = test_plan.check_templating_will_work(&HashMap::new(), &Context::new());
        assert!(
            res.is_err(),
            "expected templating will work to fail, got {:?}",
            res
        );

        let error = res.unwrap_err().unwrap_single();
        assert_eq!(
            error.kind, expected_err_kind,
            "test that the error kind is as expected"
        );
        assert_eq!(
            error.message, expected_err_message,
            "test that error message is as expected"
        );
    }

    #[test_case(&["scenario"], &[], &[], &["scenario"]; "scenario missing variables")]
    #[test_case(&[], &["setup"], &[], &["setup"]; "setup missing variables")]
    #[test_case(&[], &[], &["teardown"], &["teardown"]; "teardown missing variables")]
    #[test_case(&[], &["setup"], &["teardown"], &["setup", "teardown"]; "setup and teardown missing variables")]
    #[test_case(&["scenario"], &["setup"], &[], &["setup", "scenario"]; "scenario and setup missing variables")]
    #[test_case(&["scenario"], &[], &["teardown"], &["teardown", "scenario"]; "scenario and teardown missing variables")]
    #[test_case(&["scenario"], &["setup"], &["teardown"], &["setup", "teardown", "scenario"]; "scenario and setup and teardown missing variables")]
    #[test]
    fn check_templating_will_work_missing_variables_errors(
        scenario_fields: &[&str],
        setup_fields: &[&str],
        teardown_fields: &[&str],
        expected_err_messages: &[&str],
    ) {
        let variables = template_context(&["foo"]).variables().clone();
        let dimensions = HashMap::new();
        let include = Vec::new();
        let mut test_plan = templatable_test_plan(
            variables,
            dimensions,
            include,
            scenario_fields,
            setup_fields,
            teardown_fields,
            &[],
        );

        let res = test_plan.check_templating_will_work(&HashMap::new(), &Context::new());
        assert!(
            res.is_err(),
            "expected templating will work to fail, got {:?}",
            res
        );

        let errors = res.unwrap_err();
        assert_eq!(
            errors.iter().count(),
            expected_err_messages.len(),
            "test that the expected number of errors occur"
        );

        assert!(
            errors
                .iter()
                .all(|e| matches!(e.kind, ErrorKind::MissingVariable)),
            "expected all errors to be {:?}, got {:?}",
            ErrorKind::MissingVariable,
            errors
        );

        let error_messages: Vec<String> = errors.iter().map(|f| f.message.clone()).collect();
        let expected_err_messages: Vec<String> = expected_err_messages
            .iter()
            .map(|f| format!("  - {}: \"description\"", f))
            .collect();
        assert_eq!(
            error_messages, expected_err_messages,
            "test that the error messages are as expected"
        );
    }

    #[test]
    fn check_templating_will_work_combined_errors() {
        let variables = template_context(&["foo", "bar"]).variables().clone();
        let dimensions = dimensions_from_keys(&["foo"], 0);
        let mut test_plan =
            templatable_test_plan(variables, dimensions, vec![], &["scenario"], &[], &[], &[]);

        let mut expected_errs = ErrorBuilder::new();
        expected_errs.push(
            ErrorKind::ConflictingVariables,
            "foo",
            &["test_plan".to_string()],
        );
        expected_errs.push(
            ErrorKind::EmptyMatrixVariable,
            "foo",
            &["test_plan".to_string()],
        );
        expected_errs.push(
            ErrorKind::MissingVariable,
            "  - scenario: \"description\"",
            &["scenario.file_providers.SCENARIO.path".to_string()],
        );
        let expected_errs = expected_errs.into_result("").unwrap_err();

        let res = test_plan.check_templating_will_work(&HashMap::new(), &Context::new());
        assert!(
            res.is_err(),
            "expected templating will work to fail, got {:?}",
            res
        );
        assert_eq!(
            res.unwrap_err(),
            expected_errs,
            "test that combined errors are as expected"
        );
    }

    #[test]
    fn check_required_variables_variable_definition_defaults_count_as_required_variables() {
        let mut test_plan = TestPlanConfig {
            environment: EnvironmentConfig {
                variable_definitions: vec![
                    variable_with_default("setup", "setup"),
                    variable_with_default("teardown", "teardown"),
                ],
                execution: EnvironmentExecution::Script(ScriptEnvironment {
                    setup: CommandSection {
                        file_providers: vec![named_file_provider_with_field("setup", p("setup"))],
                        ..CommandSection::empty()
                    },
                    teardown: CommandSection {
                        file_providers: vec![named_file_provider_with_field(
                            "teardown",
                            p("teardown"),
                        )],
                        ..CommandSection::empty()
                    },
                }),
                ..EnvironmentConfig::empty()
            },
            scenario: ScenarioConfig {
                variable_definitions: vec![variable_with_default("scenario", "scenario")],
                execution: ScenarioExecution::Script(CommandSection {
                    file_providers: vec![named_file_provider_with_field("scenario", p("scenario"))],
                    ..CommandSection::empty()
                }),
                ..ScenarioConfig::empty()
            },
            ..TestPlanConfig::empty()
        };

        let res = test_plan.check_templating_will_work(&HashMap::new(), &Context::new());
        assert!(
            res.is_ok(),
            "expected templating will work to succeed, got {:?}",
            res
        );
    }

    #[test]
    fn check_templating_will_work_valid_allowed_values() {
        // Test that valid allowed_values configuration passes validation
        let mut test_plan = TestPlanConfig {
            environment: EnvironmentConfig {
                variable_definitions: vec![variable_with_allowed_values(
                    "env_var",
                    Some("a"),
                    Some(vec!["a", "b", "c"]),
                )],
                ..EnvironmentConfig::empty()
            },
            scenario: ScenarioConfig {
                variable_definitions: vec![variable_with_allowed_values(
                    "scenario_var",
                    None,
                    Some(vec!["x", "y"]),
                )],
                ..ScenarioConfig::empty()
            },
            ..TestPlanConfig::empty()
        };

        let res = test_plan.check_templating_will_work(&HashMap::new(), &Context::new());
        assert!(
            res.is_ok(),
            "expected templating will work to succeed with valid allowed_values, got {:?}",
            res
        );
    }

    #[test_case(
        vec![variable_with_allowed_values("foo", None, Some(vec![]))],
        vec![],
        vec!["environment.variable_definitions.foo"];
        "single empty allowed_values in environment"
    )]
    #[test_case(
        vec![],
        vec![variable_with_allowed_values("bar", None, Some(vec![]))],
        vec!["scenario.variable_definitions.bar"];
        "single empty allowed_values in scenario"
    )]
    #[test_case(
        vec![variable_with_allowed_values("foo", None, Some(vec![]))],
        vec![variable_with_allowed_values("bar", None, Some(vec![]))],
        vec!["environment.variable_definitions.foo", "scenario.variable_definitions.bar"];
        "empty allowed_values in both environment and scenario"
    )]
    #[test_case(
        vec![
            variable_with_allowed_values("foo", None, Some(vec![])),
            variable_with_allowed_values("bar", None, Some(vec![]))
        ],
        vec![],
        vec!["environment.variable_definitions.foo", "environment.variable_definitions.bar"];
        "multiple empty allowed_values in environment"
    )]
    #[test]
    fn check_templating_will_work_empty_allowed_values_errors(
        env_variable_defs: Vec<VariableDefinition>,
        scenario_variable_defs: Vec<VariableDefinition>,
        expected_err_paths: Vec<&str>,
    ) {
        let mut test_plan = TestPlanConfig {
            environment: EnvironmentConfig {
                variable_definitions: env_variable_defs,
                ..EnvironmentConfig::empty()
            },
            scenario: ScenarioConfig {
                variable_definitions: scenario_variable_defs,
                ..ScenarioConfig::empty()
            },
            ..TestPlanConfig::empty()
        };

        let res = test_plan.check_templating_will_work(&HashMap::new(), &Context::new());
        assert!(
            res.is_err(),
            "expected templating will work to fail for empty allowed_values, got {:?}",
            res
        );

        let errors = res.unwrap_err();
        assert_eq!(
            errors.iter().count(),
            expected_err_paths.len(),
            "expected {} errors, got {:?}",
            expected_err_paths.len(),
            errors
        );

        assert!(
            errors
                .iter()
                .all(|e| matches!(e.kind, ErrorKind::EmptyAllowedValues)),
            "expected all errors to be EmptyAllowedValues, got {:?}",
            errors
        );

        let err_paths: Vec<&str> = errors.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(
            err_paths, expected_err_paths,
            "test that error paths are as expected"
        );
    }

    #[test_case(
        vec![variable_with_allowed_values("foo", Some("c"), Some(vec!["a", "b"]))],
        vec![],
        vec![("environment.variable_definitions.foo", "variable 'foo' has default 'c' not in allowed values")];
        "single default not in allowed values in environment"
    )]
    #[test_case(
        vec![],
        vec![variable_with_allowed_values("bar", Some("z"), Some(vec!["x", "y"]))],
        vec![("scenario.variable_definitions.bar", "variable 'bar' has default 'z' not in allowed values")];
        "single default not in allowed values in scenario"
    )]
    #[test_case(
        vec![variable_with_allowed_values("foo", Some("c"), Some(vec!["a", "b"]))],
        vec![variable_with_allowed_values("bar", Some("z"), Some(vec!["x", "y"]))],
        vec![
            ("environment.variable_definitions.foo", "variable 'foo' has default 'c' not in allowed values"),
            ("scenario.variable_definitions.bar", "variable 'bar' has default 'z' not in allowed values")
        ];
        "default not in allowed values in both environment and scenario"
    )]
    #[test]
    fn check_templating_will_work_default_not_in_allowed_values_errors(
        env_variable_defs: Vec<VariableDefinition>,
        scenario_variable_defs: Vec<VariableDefinition>,
        expected_errs: Vec<(&str, &str)>,
    ) {
        let mut test_plan = TestPlanConfig {
            environment: EnvironmentConfig {
                variable_definitions: env_variable_defs,
                ..EnvironmentConfig::empty()
            },
            scenario: ScenarioConfig {
                variable_definitions: scenario_variable_defs,
                ..ScenarioConfig::empty()
            },
            ..TestPlanConfig::empty()
        };

        let res = test_plan.check_templating_will_work(&HashMap::new(), &Context::new());
        assert!(
            res.is_err(),
            "expected templating will work to fail for default not in allowed_values, got {:?}",
            res
        );

        let errors = res.unwrap_err();
        assert_eq!(
            errors.iter().count(),
            expected_errs.len(),
            "expected {} errors, got {:?}",
            expected_errs.len(),
            errors
        );

        assert!(
            errors
                .iter()
                .all(|e| matches!(e.kind, ErrorKind::DefaultNotInAllowedValues)),
            "expected all errors to be DefaultNotInAllowedValues, got {:?}",
            errors
        );

        for (error, (expected_path, expected_msg_prefix)) in errors.iter().zip(expected_errs.iter())
        {
            assert_eq!(
                error.path, *expected_path,
                "test that error path is as expected"
            );
            assert!(
                error.message.starts_with(expected_msg_prefix),
                "expected message to start with '{}', got '{}'",
                expected_msg_prefix,
                error.message
            );
        }
    }

    #[test]
    fn check_templating_will_work_combined_allowed_values_errors() {
        // Test that empty allowed_values and default not in allowed_values are both caught
        let mut test_plan = TestPlanConfig {
            environment: EnvironmentConfig {
                variable_definitions: vec![
                    variable_with_allowed_values("empty", None, Some(vec![])),
                    variable_with_allowed_values(
                        "invalid_default",
                        Some("c"),
                        Some(vec!["a", "b"]),
                    ),
                ],
                ..EnvironmentConfig::empty()
            },
            scenario: ScenarioConfig {
                variable_definitions: vec![variable_with_allowed_values(
                    "valid",
                    Some("x"),
                    Some(vec!["x", "y"]),
                )],
                ..ScenarioConfig::empty()
            },
            ..TestPlanConfig::empty()
        };

        let res = test_plan.check_templating_will_work(&HashMap::new(), &Context::new());
        assert!(
            res.is_err(),
            "expected templating will work to fail, got {:?}",
            res
        );

        let errors = res.unwrap_err();
        assert_eq!(
            errors.iter().count(),
            2,
            "expected 2 errors (empty and invalid_default), got {:?}",
            errors
        );

        let err_kinds: Vec<_> = errors.iter().map(|e| &e.kind).collect();
        assert!(
            err_kinds.contains(&&ErrorKind::EmptyAllowedValues),
            "expected EmptyAllowedValues error, got {:?}",
            err_kinds
        );
        assert!(
            err_kinds.contains(&&ErrorKind::DefaultNotInAllowedValues),
            "expected DefaultNotInAllowedValues error, got {:?}",
            err_kinds
        );
    }

    #[test]
    fn check_templating_will_work_custom_provider_valid_allowed_values() {
        let (mut test_plan, ctx) =
            test_plan_with_custom_provider_definitions(vec![variable_with_allowed_values(
                "env_type",
                Some("dev"),
                Some(vec!["dev", "staging", "prod"]),
            )]);

        let res = test_plan.check_templating_will_work(&HashMap::new(), &ctx);
        assert!(
            res.is_ok(),
            "expected templating will work to succeed with valid custom provider allowed_values, got {:?}",
            res
        );
    }

    #[test]
    fn check_templating_will_work_custom_provider_empty_allowed_values_errors() {
        // Use None for default to avoid also triggering DefaultNotInAllowedValues
        let (mut test_plan, ctx) =
            test_plan_with_custom_provider_definitions(vec![variable_with_allowed_values(
                "env_type",
                None,
                Some(vec![]),
            )]);

        let res = test_plan.check_templating_will_work(&HashMap::new(), &ctx);
        assert!(
            res.is_err(),
            "expected templating will work to fail for custom provider with empty allowed_values, got {:?}",
            res
        );

        let errors = res.unwrap_err();
        assert_eq!(
            errors.iter().count(),
            1,
            "expected 1 error, got {:?}",
            errors
        );

        let error = errors.iter().next().unwrap();
        assert!(
            matches!(error.kind, ErrorKind::EmptyAllowedValues),
            "expected EmptyAllowedValues error, got {:?}",
            error.kind
        );
        assert_eq!(
            error.path, "custom_providers.test_provider.variable_definitions.env_type",
            "expected error path to reference custom provider"
        );
    }

    #[test]
    fn check_templating_will_work_custom_provider_default_not_in_allowed_values_errors() {
        let (mut test_plan, ctx) =
            test_plan_with_custom_provider_definitions(vec![variable_with_allowed_values(
                "env_type",
                Some("test"),
                Some(vec!["dev", "staging", "prod"]),
            )]);

        let res = test_plan.check_templating_will_work(&HashMap::new(), &ctx);
        assert!(
            res.is_err(),
            "expected templating will work to fail for custom provider with default not in allowed_values, got {:?}",
            res
        );

        let errors = res.unwrap_err();
        assert_eq!(
            errors.iter().count(),
            1,
            "expected 1 error, got {:?}",
            errors
        );

        let error = errors.iter().next().unwrap();
        assert!(
            matches!(error.kind, ErrorKind::DefaultNotInAllowedValues),
            "expected DefaultNotInAllowedValues error, got {:?}",
            error.kind
        );
        assert_eq!(
            error.path, "custom_providers.test_provider.variable_definitions.env_type",
            "expected error path to reference custom provider"
        );
        assert!(
            error.message.contains("env_type"),
            "expected error message to contain variable name, got '{}'",
            error.message
        );
    }

    #[test]
    fn check_templating_will_work_compatible_allowed_values_across_configs() {
        // Scenario and environment both define 'foo' with overlapping allowed_values
        let mut test_plan = TestPlanConfig {
            variables: [("foo".into(), "a".into())].into(),
            environment: EnvironmentConfig {
                variable_definitions: vec![variable_with_allowed_values(
                    "foo",
                    None,
                    Some(vec!["a", "b", "c"]),
                )],
                ..EnvironmentConfig::empty()
            },
            scenario: ScenarioConfig {
                variable_definitions: vec![variable_with_allowed_values(
                    "foo",
                    None,
                    Some(vec!["a", "b", "d"]),
                )],
                ..ScenarioConfig::empty()
            },
            ..TestPlanConfig::empty()
        };

        let res = test_plan.check_templating_will_work(&HashMap::new(), &Context::new());
        assert!(
            res.is_ok(),
            "expected compatible allowed_values to succeed, got {:?}",
            res
        );
    }

    #[test]
    fn check_templating_will_work_incompatible_allowed_values_errors() {
        // Scenario and environment both define 'foo' with NO overlapping allowed_values
        let mut test_plan = TestPlanConfig {
            environment: EnvironmentConfig {
                variable_definitions: vec![variable_with_allowed_values(
                    "foo",
                    None,
                    Some(vec!["a", "b"]),
                )],
                ..EnvironmentConfig::empty()
            },
            scenario: ScenarioConfig {
                variable_definitions: vec![variable_with_allowed_values(
                    "foo",
                    None,
                    Some(vec!["c", "d"]),
                )],
                ..ScenarioConfig::empty()
            },
            ..TestPlanConfig::empty()
        };

        let res = test_plan.check_templating_will_work(&HashMap::new(), &Context::new());
        assert!(res.is_err(), "expected incompatible allowed_values to fail");

        let errors = res.unwrap_err();
        assert!(
            errors
                .iter()
                .any(|e| matches!(e.kind, ErrorKind::IncompatibleAllowedValues)),
            "expected IncompatibleAllowedValues error, got {:?}",
            errors
        );
        assert!(
            errors.iter().any(|e| e.message.contains("foo")),
            "expected error message to contain variable name 'foo'"
        );
    }

    #[test_case(
        Matrix {
            dimensions: [("foo".into(), vec!["x".into(), "y".into()])].into(),
            ..Default::default()
        },
        "dimensions";
        "matrix dimension value not allowed"
    )]
    #[test_case(
        Matrix {
            include: vec![[("foo".into(), "x".into())].into()],
            ..Default::default()
        },
        "include";
        "matrix include value not allowed"
    )]
    #[test]
    fn check_templating_will_work_matrix_value_not_allowed(
        matrix: Matrix,
        expected_path_part: &str,
    ) {
        let mut test_plan = TestPlanConfig {
            matrix,
            scenario: ScenarioConfig {
                variable_definitions: vec![variable_with_allowed_values(
                    "foo",
                    None,
                    Some(vec!["a", "b"]),
                )],
                ..ScenarioConfig::empty()
            },
            ..TestPlanConfig::empty()
        };

        let res = test_plan.check_templating_will_work(&HashMap::new(), &Context::new());
        assert!(
            res.is_err(),
            "expected matrix {} with invalid value to fail",
            expected_path_part
        );

        let errors = res.unwrap_err();
        assert!(
            errors
                .iter()
                .any(|e| matches!(e.kind, ErrorKind::ValueNotAllowed)),
            "expected ValueNotAllowed error, got {:?}",
            errors
        );
        assert!(
            errors.iter().any(|e| e.path.contains(expected_path_part)),
            "expected error path to contain '{}', got {:?}",
            expected_path_part,
            errors
        );
    }

    #[test_case(
        false,
        "test plan variable";
        "test plan variable not allowed"
    )]
    #[test_case(
        true,
        "CLI variable";
        "CLI variable not allowed"
    )]
    #[test]
    fn check_templating_will_work_variable_not_allowed(
        is_cli_override: bool,
        expected_source: &str,
    ) {
        let mut test_plan = TestPlanConfig {
            variables: [("foo".into(), "x".into())].into(),
            scenario: ScenarioConfig {
                variable_definitions: vec![variable_with_allowed_values(
                    "foo",
                    None,
                    Some(vec!["a", "b"]),
                )],
                ..ScenarioConfig::empty()
            },
            ..TestPlanConfig::empty()
        };

        let variable_sources: HashMap<String, StableSource> = if is_cli_override {
            [("foo".into(), StableSource::Cli)].into()
        } else {
            HashMap::new()
        };

        let res = test_plan.check_templating_will_work(&variable_sources, &Context::new());
        assert!(
            res.is_err(),
            "expected {} with invalid value to fail",
            expected_source
        );

        let errors = res.unwrap_err();
        assert!(
            errors
                .iter()
                .any(|e| matches!(e.kind, ErrorKind::ValueNotAllowed)),
            "expected ValueNotAllowed error, got {:?}",
            errors
        );
        assert!(
            errors.iter().any(|e| e.message.contains(expected_source)),
            "expected error message to identify source as {}, got {:?}",
            expected_source,
            errors
        );
    }
}
