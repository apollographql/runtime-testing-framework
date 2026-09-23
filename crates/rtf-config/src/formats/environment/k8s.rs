use crate::{
    checks::{self, Check, CheckArrayDuplicates, DedupArray},
    context::ResolutionContext,
    formats::environment::manifest::{ManifestEnvironment, NamedManifestFiles},
    inlining::{self, Inline, InlineMode, InlinedProvider},
    providers::file::manifest::NamedManifestFileProvider,
    run::{Provider, RunProviders, ValidateEnvironment},
};
use rtf_derive::Template;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, pin::Pin};

pub type K8sEnvironment = ManifestEnvironment<K8sResources>;

impl ValidateEnvironment for K8sEnvironment {}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, Template)]
pub struct K8sResources {
    /// A list of all the kubernetes resource files to apply for this environment
    pub resources: Vec<NamedManifestFileProvider>,
}

impl ValidateEnvironment for K8sResources {}

impl NamedManifestFiles for K8sResources {
    fn manifest_files(&self) -> &Vec<NamedManifestFileProvider> {
        &self.resources
    }

    fn manifest_files_mut(&mut self) -> &mut Vec<NamedManifestFileProvider> {
        &mut self.resources
    }
}

impl RunProviders for K8sResources {
    fn named_providers<'a>(&'a self) -> Vec<(&'a str, Provider<'a>)> {
        self.resources.named_providers()
    }
}

impl Inline for K8sResources {
    fn try_inline<'a>(
        &'a mut self,
        mode: InlineMode,
        ctx: &'a impl ResolutionContext,
        cache: &'a mut HashMap<u64, InlinedProvider>,
    ) -> Pin<Box<dyn Future<Output = inlining::Result<()>> + Send + 'a>> {
        self.resources.try_inline(mode, ctx, cache)
    }
}

impl Check for K8sResources {
    fn try_check(
        &self,
        path: &mut Vec<String>,
        ctx: &impl ResolutionContext,
    ) -> checks::Result<()> {
        let mut errs = checks::ErrorBuilder::new();

        for ncfp in self.resources.iter() {
            errs.append(ncfp.try_check_nested(path, "resources", ctx));
        }

        errs.into_result(())
    }
}

impl CheckArrayDuplicates for K8sResources {
    const BASE_PATH: &str = "k8s_environment";

    fn deduplicated_arrays<'a>(&'a mut self) -> Vec<(&'static str, DedupArray<'a>)> {
        vec![("resources", DedupArray::Ncfp(&mut self.resources))]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        checks::ErrorKind,
        context::Context,
        formats::{
            EnvironmentConfig, OutputCollection, Sources,
            tests::{
                assert_check_errors, templatable_file_providers, template_context,
                variable_definitions,
            },
        },
        inlining::InlineMode,
        providers::{
            file::{
                FileProvider, InlineFile, NamedFileProvider, RelativeFile, RequiredFile, SourceDir,
                StableSource, manifest::ManifestFileProvider,
            },
            test_helpers::create_temp_dir_with_file,
        },
        templating::{Field, Template},
    };
    use assert_fs::{fixture::PathChild, prelude::FileWriteStr};
    use std::collections::HashMap;

    /// Create an empty [EnvironmentConfig] wrapping a [K8sEnvironment] for tests.
    fn empty_k8s_environment_config(
        resources: Vec<NamedManifestFileProvider>,
    ) -> EnvironmentConfig<K8sEnvironment> {
        EnvironmentConfig {
            name: String::new(),
            description: String::new(),
            variable_definitions: Vec::new(),
            custom_providers: Vec::new(),
            execution: K8sEnvironment {
                resources: K8sResources { resources },
                file_providers: Vec::new(),
                env_vars: HashMap::new(),
                output_collection: OutputCollection {
                    prometheus: Vec::new(),
                },
            },
        }
    }

    fn named_k8s_resource(name: &str) -> NamedManifestFileProvider {
        NamedManifestFileProvider {
            name: name.to_string(),
            provider: ManifestFileProvider::Inline(InlineFile {
                content: format!("# {name}\nkind: Deployment"),
            }),
        }
    }

    #[test]
    fn try_template_k8s_succeeds() {
        let field_names = &["resource", "file"];
        let ctx = template_context(field_names);

        let mut environment = empty_k8s_environment_config(vec![NamedManifestFileProvider {
            name: "resource.yaml".to_string(),
            provider: ManifestFileProvider::RelativePath(RelativeFile {
                path: Field::Pending("resource".to_string()),
                src: None,
            }),
        }]);
        environment.variable_definitions = variable_definitions(field_names);
        environment.execution.file_providers = templatable_file_providers(&["file"]);

        let res = environment.try_template(&mut Vec::new(), &StableSource::Environment, &ctx);
        assert!(
            res.is_ok(),
            "expected to template successfully, got {res:?}"
        )
    }

    #[tokio::test]
    async fn k8s_environment_config_inline_succeeds() {
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

        let relative_resource = ManifestFileProvider::RelativePath(RelativeFile {
            path: Field::Resolved("file.txt".to_string()),
            src: Some(StableSource::Environment),
        });

        let mut environment = empty_k8s_environment_config(vec![NamedManifestFileProvider {
            name: "resource.yaml".to_string(),
            provider: relative_resource,
        }]);
        environment.execution.file_providers = vec![NamedFileProvider {
            name: "file.txt".to_string(),
            env_var: "FILE".to_string(),
            provider: FileProvider::RelativePath(RelativeFile {
                path: Field::Resolved("file.txt".to_string()),
                src: Some(StableSource::Environment),
            }),
        }];

        let result = environment
            .try_inline(InlineMode::All, &ctx, &mut HashMap::new())
            .await;

        assert!(result.is_ok(), "Expected inline to succeed, got {result:?}");

        let expected_inline_resource = NamedManifestFileProvider {
            name: "resource.yaml".to_string(),
            provider: ManifestFileProvider::Inline(InlineFile {
                content: "example file content".to_string(),
            }),
        };

        assert_eq!(
            environment.execution.resources.resources[0], expected_inline_resource,
            "Expected resource to be inlined"
        );
    }

    #[test]
    fn check_k8s_success() {
        let environment = empty_k8s_environment_config(vec![NamedManifestFileProvider {
            name: "resource.yaml".to_string(),
            provider: ManifestFileProvider::Inline(InlineFile {
                content: "content".to_string(),
            }),
        }]);

        let ctx = Context::new();

        let res = environment.try_check(&mut Vec::new(), &ctx);
        assert!(res.is_ok(), "expected check to succeed, got {res:?}");
    }

    #[test]
    fn try_check_k8s_resource_errors() {
        let environment = empty_k8s_environment_config(vec![NamedManifestFileProvider {
            name: "resource.yaml".to_string(),
            provider: ManifestFileProvider::Required(RequiredFile {
                message: "this is a required file".to_string(),
            }),
        }]);

        let ctx = Context::new();

        assert_check_errors(environment, &ctx, &[ErrorKind::RequiredFileMissing]);
    }

    #[tokio::test]
    async fn manifest_file_paths_resolves_registered_resources() {
        let env = empty_k8s_environment_config(vec![
            named_k8s_resource("one.yaml"),
            named_k8s_resource("two.yaml"),
        ])
        .execution;
        let mut ctx = Context::new();

        let temp_dir = assert_fs::TempDir::new().unwrap();
        let mut expected_paths = Vec::new();

        for ncfp in env.resources.resources.iter() {
            let file = temp_dir.child(&ncfp.name);
            file.write_str(&ncfp.name).unwrap();
            let path = file.path().to_path_buf();
            ctx.store_provider_output_path(
                Provider::ComposeFile { fp: &ncfp.provider },
                path.clone(),
            );
            expected_paths.push(path);
        }

        let paths = env
            .manifest_file_paths(&ctx)
            .expect("expected manifest file paths to resolve");

        assert_eq!(paths, expected_paths);
    }
}
