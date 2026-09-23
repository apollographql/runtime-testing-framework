use crate::{
    checks::{self, Check, CheckArrayDuplicates, DedupArray},
    context::ResolutionContext,
    inlining::{self, Inline, InlineMode, InlinedProvider},
    providers::{self, command::CommandSection},
    run::{Execute, Provider, RunEnvironment, RunProviders, ValidateEnvironment},
};
use rtf_derive::Template;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::Path, pin::Pin};

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, Template)]
pub struct ScriptEnvironment {
    pub setup: CommandSection,
    pub teardown: CommandSection,
}

impl ValidateEnvironment for ScriptEnvironment {}

impl RunEnvironment for ScriptEnvironment {
    async fn execute_setup(
        &self,
        name: &str,
        out_dir: &Path,
        ctx: &mut impl ResolutionContext,
    ) -> providers::Result<String> {
        self.setup
            .run_providers_and_execute_for_output(name, out_dir, ctx)
            .await
    }

    async fn execute_teardown(
        &self,
        name: &str,
        out_dir: &Path,
        ctx: &mut impl ResolutionContext,
    ) -> providers::Result<String> {
        self.teardown
            .run_providers_and_execute_for_output(name, out_dir, ctx)
            .await
    }
}

impl Check for ScriptEnvironment {
    fn try_check(
        &self,
        path: &mut Vec<String>,
        ctx: &impl ResolutionContext,
    ) -> checks::Result<()> {
        let mut errs = checks::ErrorBuilder::new();

        errs.append(self.setup.try_check_nested(path, "setup", ctx));
        errs.append(self.teardown.try_check_nested(path, "teardown", ctx));

        errs.into_result(())
    }
}

impl RunProviders for ScriptEnvironment {
    fn named_providers<'a>(&'a self) -> Vec<(&'a str, Provider<'a>)> {
        let mut providers = self.setup.named_providers();
        providers.extend(self.teardown.named_providers());

        providers
    }
}

impl Inline for ScriptEnvironment {
    fn try_inline<'a>(
        &'a mut self,
        mode: InlineMode,
        ctx: &'a impl ResolutionContext,
        cache: &'a mut HashMap<u64, InlinedProvider>,
    ) -> Pin<Box<dyn Future<Output = inlining::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let mut errs = inlining::ErrorBuilder::new();

            errs.append(self.setup.try_inline(mode, ctx, cache).await);
            errs.append(self.teardown.try_inline(mode, ctx, cache).await);

            errs.into_result(())
        })
    }
}

impl CheckArrayDuplicates for ScriptEnvironment {
    const BASE_PATH: &str = "script_environment";

    fn deduplicated_arrays<'a>(&'a mut self) -> Vec<(&'static str, DedupArray<'a>)> {
        vec![
            (
                "setup.file_providers",
                DedupArray::Nfp(&mut self.setup.file_providers),
            ),
            (
                "teardown.file_providers",
                DedupArray::Nfp(&mut self.teardown.file_providers),
            ),
        ]
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::{
        StableSource,
        context::Context,
        formats::{
            Sources,
            environment::{
                EnvironmentConfig, EnvironmentExecution, test_helpers::templatable_environment,
            },
            tests::{assert_template_errors, expected_error_details, template_context},
        },
        providers::{
            command::{
                CommandProvider, CommandSection, CommandSpec, test_helpers::cmd_with_inline_file,
            },
            file::{InlineFile, RawSource, RelativeFile, SourceDir},
            test_helpers::create_temp_dir_with_file,
        },
        templating::{Field, Template, TemplateContext},
    };
    use indoc::indoc;
    use simple_test_case::test_case;
    use std::{collections::HashMap, path::PathBuf};

    // An example environment config to check parsing and templating
    const TEMPLATED_SCRIPT_ENVIRONMENT: &str = indoc!(
        r#"
        name: environment
        description: a templated environment
        variable_definitions:
          - name: foo
            description: a value foo
            allowed_values: ["foo1", "foo2"]
          - name: bar
            description: a value bar
            default: "bar"
        custom_providers:
          - kind: local
            relative_path: ../providers
            using:
              my_custom_provider: my_custom_provider.yaml
          - kind: github
            org: apollographql
            repo: test-providers
            path: /providers
            git_ref: main
            using:
              another_provider: another_provider.yaml
        setup:
          command:
            name: setup.sh
            kind: relative_path
            path: "/setup.sh"
          file_providers:
            - name: foo.txt
              env_var: FOO
              kind: relative_path
              path: "{{ foo }}"
        teardown:
          command:
            name: teardown.sh
            kind: relative_path
            path: "/teardown.sh"
          file_providers:
            - name: bar.txt
              env_var: BAR
              kind: relative_path
              path: "{{ bar }}"
    "#
    );

    #[test]
    fn parse_script_environment_success() {
        let config: EnvironmentConfig<EnvironmentExecution> =
            serde_yaml::from_str(TEMPLATED_SCRIPT_ENVIRONMENT)
                .expect("environment config to parse");

        let mut res = config.required_variables();
        res.sort(); // Sorting so variables are in a deterministic order for the assert_eq

        assert_eq!(res, &["bar", "foo"], "expected variables to match");
        assert_eq!(config.custom_providers.len(), 2);

        let cp = &config.custom_providers[0];
        assert_eq!(
            cp.source,
            RawSource::Local {
                relative_path: PathBuf::from("../providers")
            }
        );
        assert_eq!(cp.using.len(), 1);
        assert_eq!(
            cp.using.get("my_custom_provider").unwrap(),
            "my_custom_provider.yaml",
        );

        let cp = &config.custom_providers[1];
        assert_eq!(
            cp.source,
            RawSource::Github {
                org: "apollographql".to_string(),
                repo: "test-providers".to_string(),
                path: PathBuf::from("/providers"),
                git_ref: Some("main".to_string())
            }
        );
        assert_eq!(cp.using.len(), 1);
        assert_eq!(
            cp.using.get("another_provider").unwrap(),
            "another_provider.yaml",
        );
    }

    #[test_case(&["setup1", "setup2"], &["teardown1", "teardown2"]; "setup multi variable and teardown multi variable")]
    #[test_case(&["setup1", "setup2"], &["teardown1"]; "setup multi variable and teardown single variable")]
    #[test_case(&["setup1", "setup2"], &[]; "setup multi variable and teardown no variable")]
    #[test_case(&["setup1"], &["teardown1", "teardown2"]; "setup single variable and teardown multi variable")]
    #[test_case(&["setup1"], &["teardown1"]; "setup single variable and teardown single variable")]
    #[test_case(&["setup1"], &[]; "setup single variable and teardown no variable")]
    #[test_case(&[], &["teardown1", "teardown2"]; "setup no variable and teardown multi variable")]
    #[test_case(&[], &["teardown1"]; "setup no variable and teardown single variable")]
    #[test_case(&[], &[]; "setup no variable and teardown no variable")]
    #[test]
    fn try_template_script_succeeds(setup_fields: &[&str], teardown_fields: &[&str]) {
        let mut field_names: Vec<&str> = setup_fields.to_vec();
        field_names.extend_from_slice(teardown_fields);

        let ctx = template_context(field_names.as_slice());
        let mut environment =
            templatable_environment(field_names.as_slice(), setup_fields, teardown_fields, &[]);

        let res = environment.try_template(&mut Vec::new(), &StableSource::Environment, &ctx);
        assert!(
            res.is_ok(),
            "expected to template successfully, got {res:?}"
        )
    }

    /// Helper function for asserting template errors are as expected
    fn assert_env_template_errors(
        environment: &mut EnvironmentConfig<EnvironmentExecution>,
        ctx: TemplateContext,
        expected_setup_err_fields: &[&str],
        expected_teardown_err_fields: &[&str],
    ) {
        let (mut expected_err_messages, mut expected_err_paths) =
            expected_error_details(expected_setup_err_fields, "setup");
        let (expected_messages, expected_paths) =
            expected_error_details(expected_teardown_err_fields, "teardown");
        expected_err_messages.extend(expected_messages);
        expected_err_paths.extend(expected_paths);

        assert_template_errors(environment, ctx, expected_err_messages, expected_err_paths);
    }

    #[test_case(&["missing"], &["setup"], &["setup"]; "single field defined and missing definition")]
    #[test_case(&["missing1", "missing2"], &["setup1", "setup2"], &["setup1", "setup2"]; "multiple fields defined and both missing definition")]
    #[test_case(&["setup1", "missing2"], &["setup1", "setup2"], &["setup2"]; "multiple fields defined and one missing definition")]
    #[test_case(&["not_provided"], &["not_provided"], &["not_provided"]; "single field defined with definition but variable not provided")]
    #[test_case(&["not_provided1", "not_provided2"], &["not_provided1", "not_provided2"], &["not_provided1", "not_provided2"]; "multiple fields defined with definition but variables not provided")]
    #[test]
    fn try_template_setup_missing_variable_definitions(
        variable_defs: &[&str],
        setup_fields: &[&str],
        expected_err_fields: &[&str],
    ) {
        let ctx = template_context(&["setup", "setup1", "setup2"]);
        let mut environment = templatable_environment(variable_defs, setup_fields, &[], &[]);

        assert_env_template_errors(&mut environment, ctx, expected_err_fields, &[]);
    }

    #[test_case(&["missing"], &["teardown"], &["teardown"]; "single field defined and missing definition")]
    #[test_case(&["missing1", "missing2"], &["teardown1", "teardown2"], &["teardown1", "teardown2"]; "multiple fields defined and both missing definition")]
    #[test_case(&["teardown1", "missing2"], &["teardown1", "teardown2"], &["teardown2"]; "multiple fields defined and one missing definition")]
    #[test_case(&["not_provided"], &["not_provided"], &["not_provided"]; "single field defined with definition but variable not provided")]
    #[test_case(&["not_provided1", "not_provided2"], &["not_provided1", "not_provided2"], &["not_provided1", "not_provided2"]; "multiple fields defined with definition but variables not provided")]
    #[test]
    fn try_template_teardown_missing_variable_definitions(
        variable_defs: &[&str],
        teardown_fields: &[&str],
        expected_err_fields: &[&str],
    ) {
        let ctx = template_context(&["teardown", "teardown1", "teardown2"]);
        let mut environment = templatable_environment(variable_defs, &[], teardown_fields, &[]);

        assert_env_template_errors(&mut environment, ctx, &[], expected_err_fields);
    }

    #[test]
    fn try_template_missing_setup_and_teardown_variable_definitions() {
        let ctx = template_context(&["setup", "teardown"]);
        let mut environment = templatable_environment(&[], &["setup"], &["teardown"], &[]);

        assert_env_template_errors(&mut environment, ctx, &["setup"], &["teardown"]);
    }

    #[test]
    fn try_template_missing_setup_and_teardown_variables_not_provided() {
        let ctx = template_context(&[]);
        let mut environment =
            templatable_environment(&["setup", "teardown"], &["setup"], &["teardown"], &[]);

        assert_env_template_errors(&mut environment, ctx, &["setup"], &["teardown"]);
    }

    // The success test is here to complete the matrix of failure tests below (i.e. no failure)
    // It shows all parts of the env config that *could* fail not failing. In reality we could
    // just use an "empty" config here and get the same result but this is a more illustrative
    // example
    #[test]
    fn check_script_success() {
        let environment = EnvironmentConfig {
            execution: EnvironmentExecution::Script(ScriptEnvironment {
                setup: cmd_with_inline_file(),
                teardown: cmd_with_inline_file(),
            }),
            ..EnvironmentConfig::empty()
        };

        let ctx = Context::new();

        let res = environment.try_check(&mut Vec::new(), &ctx);
        assert!(res.is_ok(), "expected check to succeed, got {res:?}");
    }

    #[tokio::test]
    async fn script_environment_config_inline_succeeds() {
        let file_content = "example file content";
        let (temp, _file_to_read) = create_temp_dir_with_file("file.txt", file_content);

        let mut ctx = Context::new();
        let src = SourceDir::local(ctx.canonicalize_path(temp.path()).unwrap());
        ctx.set_sources(Sources::with_custom_providers(
            SourceDir::default(),
            None,
            Some(src.clone()),
            Default::default(),
            Default::default(),
        ));

        let relative_command_provider = CommandProvider::RelativePath(RelativeFile {
            path: Field::Resolved("file.txt".to_string()),
            src: Some(StableSource::Environment),
        });

        let mut environment = EnvironmentConfig {
            execution: EnvironmentExecution::Script(ScriptEnvironment {
                setup: CommandSection {
                    command: CommandSpec {
                        name: "setup.sh".to_string(),
                        command_provider: relative_command_provider.clone(),
                        args: Vec::new(),
                    },
                    ..CommandSection::empty()
                },
                teardown: CommandSection {
                    command: CommandSpec {
                        name: "teardown.sh".to_string(),
                        command_provider: relative_command_provider,
                        args: Vec::new(),
                    },
                    ..CommandSection::empty()
                },
            }),
            ..EnvironmentConfig::empty()
        };

        let result = environment
            .try_inline(InlineMode::All, &ctx, &mut HashMap::new())
            .await;

        assert!(result.is_ok(), "Expected inline to succeed, got {result:?}");

        let expected_inline = CommandProvider::Inline(InlineFile {
            content: file_content.to_string(),
        });
        let script = match &environment.execution {
            EnvironmentExecution::Script(script) => script,
            _ => panic!("Expected EnvironmentExecution::Script variant"),
        };

        assert_eq!(
            script.setup.command.command_provider, expected_inline,
            "Expected setup command provider to be inlined"
        );
        assert_eq!(
            script.teardown.command.command_provider, expected_inline,
            "Expected teardown command provider to be inlined"
        );
    }
}
