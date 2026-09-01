use crate::{
    FILE_PROVIDERS_LABEL,
    checks::{self, Check, CheckArrayDuplicates, DedupArray},
    context::ResolutionContext,
    formats::environment::manifest::{ManifestEnvironment, NamedManifestFiles},
    inlining::{self, InlineMode, InlinedProvider},
    providers::{
        self,
        file::{
            InlineDir, InlineFile,
            compose::{ComposeFileProvider, NamedComposeFileProvider},
        },
    },
    run::{
        DOCKER_COMPOSE_NETWORK, OUTPUT_PATH, PROVIDER_DIR, Provider, RunEnvironment, RunProviders,
        ValidateEnvironment,
    },
};
use rtf_derive::Template;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_yaml::{Mapping, Value};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::{Path, PathBuf},
    pin::Pin,
};
use tracing::warn;

const PROVIDERS_CONTAINER_PATH: &str = "/providers";

/// The label kompose reads to set a container's `imagePullPolicy`.
///
/// kompose does not read compose's native `pull_policy` field itself (tracked upstream at
/// https://github.com/kubernetes/kompose/issues/1923, open and unresolved) -- it only derives
/// `imagePullPolicy` from this label.
const KOMPOSE_IMAGE_PULL_POLICY_LABEL: &str = "kompose.image-pull-policy";

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, Template)]
pub struct ComposeResources {
    /// The name of the docker compose project. Defaults to the environment name if not set.
    #[serde(default)]
    #[template(skip)]
    pub project_name: Option<String>,
    /// A list of all the docker compose files to start for this environment
    pub compose_files: Vec<NamedComposeFileProvider>,
}

impl ComposeResources {
    fn project_name(&self) -> String {
        let raw = self.project_name.as_deref().unwrap_or("rtf_environment");
        slugify_compose_project_name(raw)
    }
}

impl ValidateEnvironment for ComposeResources {}

impl NamedManifestFiles for ComposeResources {
    fn manifest_files(&self) -> &Vec<NamedComposeFileProvider> {
        &self.compose_files
    }

    fn manifest_files_mut(&mut self) -> &mut Vec<NamedComposeFileProvider> {
        &mut self.compose_files
    }
}

impl RunProviders for ComposeResources {
    fn named_providers<'a>(&'a self) -> Vec<(&'a str, Provider<'a>)> {
        self.compose_files.named_providers()
    }

    fn inline<'a>(
        &'a mut self,
        mode: &'a InlineMode,
        ctx: &'a impl ResolutionContext,
        cache: &'a mut HashMap<u64, InlinedProvider>,
    ) -> Pin<Box<dyn Future<Output = inlining::Result<()>> + Send + 'a>> {
        Box::pin(async move { self.compose_files.inline(mode, ctx, cache).await })
    }
}

impl Check for ComposeResources {
    fn try_check(
        &self,
        path: &mut Vec<String>,
        ctx: &impl ResolutionContext,
    ) -> checks::Result<()> {
        let mut errs = checks::ErrorBuilder::new();

        for ncfp in self.compose_files.iter() {
            errs.append(ncfp.try_check_nested(path, "compose_files", ctx));
        }

        errs.into_result(())
    }
}

impl CheckArrayDuplicates for ComposeResources {
    const BASE_PATH: &str = "docker_compose_environment";

    fn deduplicated_arrays<'a>(&'a mut self) -> Vec<(&'static str, DedupArray<'a>)> {
        vec![("compose_files", DedupArray::Ncfp(&mut self.compose_files))]
    }
}

/// An environment provisioned from a set of docker compose files.
pub type DockerComposeEnvironment = ManifestEnvironment<ComposeResources>;

impl DockerComposeEnvironment {
    fn setup_as_command_and_args(
        &self,
        project_name: &str,
        compose_overrides: &[PathBuf],
        ctx: &impl ResolutionContext,
    ) -> providers::Result<(&'static str, Vec<String>)> {
        let mut args = vec![
            "compose".to_string(),
            "-p".to_string(),
            project_name.to_string(),
        ];

        for file in self.manifest_file_paths(ctx)? {
            args.push("-f".to_string());
            args.push(file.to_string_lossy().to_string());
        }

        for file in compose_overrides {
            args.push("-f".to_string());
            args.push(file.to_string_lossy().to_string());
        }

        // Add up command with flags: detached mode, wait for health checks
        args.extend(["up".to_string(), "-d".to_string(), "--wait".to_string()]);

        Ok(("docker", args))
    }

    fn teardown_as_command_and_args(
        &self,
        project_name: &str,
    ) -> providers::Result<(&'static str, Vec<String>)> {
        let mut args = vec![
            "compose".to_string(),
            "-p".to_string(),
            project_name.to_string(),
        ];

        args.push("down".to_string());

        Ok(("docker", args))
    }

    /// Categorise docker compose services based on how they accesses file provider output.
    pub fn file_provider_services(
        &self,
        ctx: &impl ResolutionContext,
    ) -> providers::Result<FileProviderServices> {
        let mut fps = FileProviderServices::default();

        if self.file_providers.is_empty() {
            return Ok(fps);
        }

        for path in self.manifest_file_paths(ctx)?.iter() {
            if let Ok(content) = ctx.read_path_to_string(path) {
                fps.add_services_from(&content);
            }
        }

        Ok(fps)
    }
}

impl ValidateEnvironment for DockerComposeEnvironment {}

impl RunEnvironment for DockerComposeEnvironment {
    async fn execute_setup(
        &self,
        name: &str,
        out_dir: &Path,
        ctx: &mut impl ResolutionContext,
    ) -> providers::Result<String> {
        let output_path = out_dir.join(OUTPUT_PATH);
        let providers_dir = out_dir.join(PROVIDER_DIR);
        let env_providers_dir = providers_dir.join(format!("{name}_providers"));

        self.run_providers(&env_providers_dir, ctx).await?;

        let fps = self.file_provider_services(ctx)?;
        fps.err_if_invalid()?;

        let mut compose_overrides = Vec::new();

        if fps.has_labeled_services() && !self.file_providers.is_empty() {
            let override_content = fps.local_file_providers_overlay(&env_providers_dir);
            let override_path = out_dir.join("file-providers-override.yaml");
            ctx.write(&override_path, override_content)?;

            compose_overrides.push(override_path);
        }

        let project_name = self.resources.project_name();
        let env_vars =
            self.build_env_vars(out_dir, &output_path, fps.has_labeled_services(), ctx)?;

        let (cmd, args) = self.setup_as_command_and_args(&project_name, &compose_overrides, ctx)?;

        ctx.run_command_blocking(cmd, args.iter().map(|s| s.as_str()), &env_vars)
            .map_err(|e| providers::Error::CommandFailed {
                name: "docker compose up".to_string(),
                err: e.to_string(),
            })?;

        ctx.store_run_metadata(DOCKER_COMPOSE_NETWORK, format!("{project_name}_default"));

        Ok("{}".to_string())
    }

    async fn execute_teardown(
        &self,
        _name: &str,
        _out_dir: &Path,
        ctx: &mut impl ResolutionContext,
    ) -> providers::Result<String> {
        let project_name = self.resources.project_name();
        let (cmd, args) = self.teardown_as_command_and_args(&project_name)?;

        ctx.run_command_blocking(cmd, args.iter().map(|s| s.as_str()), &HashMap::new())
            .map_err(|e| providers::Error::CommandFailed {
                name: "docker compose down".to_string(),
                err: e.to_string(),
            })?;

        Ok("{}".to_string())
    }
}

/// Slugify a string into a valid docker compose project name.
///
/// Project names must contain only lowercase letters, decimal digits, dashes, and underscores,
/// and must begin with a lowercase letter or decimal digit.
fn slugify_compose_project_name(name: &str) -> String {
    let mut slug: String = name
        .chars()
        .map(|c| match c {
            'A'..='Z' => c.to_ascii_lowercase(),
            'a'..='z' | '0'..='9' | '-' | '_' => c,
            _ => '-',
        })
        .collect();

    // Strip leading characters that aren't a lowercase letter or digit
    while slug.starts_with('-') || slug.starts_with('_') {
        slug.remove(0);
    }

    slug
}

/// Categorisation of compose services based on how they access file provider output.
///
/// A service may appear in both `labeled` and `explicit_mount` if it carries the label AND
/// has explicit volume mounts (statically an error state). Services that are not making
/// use of file providers do not appear at all.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct FileProviderServices {
    /// Services carrying the `rtf.io/file-providers: true` label.
    pub labeled: HashSet<String>,
    /// Services with explicit volume mounts.
    pub explicit_mount: HashSet<String>,
}

impl FileProviderServices {
    pub fn from_inline(dce: &DockerComposeEnvironment) -> Self {
        let mut fps = Self::default();

        for fp in dce.resources.compose_files.iter() {
            match &fp.provider {
                ComposeFileProvider::Inline(InlineFile { content }) => {
                    fps.add_services_from(content);
                }

                ComposeFileProvider::InlineDir(InlineDir { files }) => {
                    for file in files.iter() {
                        fps.add_services_from(&file.content);
                    }
                }

                _ => warn!(
                    "FileProviderServices::from_inline called on non-inline provider {}",
                    fp.name
                ),
            }
        }

        fps
    }

    pub fn err_if_invalid(&self) -> providers::Result<()> {
        if self.has_labeled_services() && self.has_explicit_volume_mounts() {
            let mut labeled: Vec<String> = self.labeled.clone().into_iter().collect();
            let mut explicit_mount: Vec<String> = self.explicit_mount.clone().into_iter().collect();
            labeled.sort_unstable();
            explicit_mount.sort_unstable();

            return Err(providers::Error::InvalidFileProviderUsage {
                labeled,
                explicit_mount,
            });
        }

        Ok(())
    }

    fn has_labeled_services(&self) -> bool {
        !self.labeled.is_empty()
    }

    fn has_explicit_volume_mounts(&self) -> bool {
        !self.explicit_mount.is_empty()
    }

    fn add_services_from(&mut self, yaml_content: &str) -> Option<()> {
        let value = serde_yaml::from_str::<Value>(yaml_content).ok()?;
        let services = value.get("services").and_then(|s| s.as_mapping())?;

        for (name, service) in services.into_iter() {
            self.add_service(name, service);
        }

        Some(())
    }

    fn add_service(&mut self, name: &Value, service: &Value) -> Option<()> {
        let name = name.as_str().map(str::to_string)?;
        let has_explicit_volumes = service.get("volumes").is_some();
        let label = service
            .get("labels")
            .and_then(|l| l.get(FILE_PROVIDERS_LABEL));

        let has_label = match label {
            Some(Value::Bool(b)) => *b,
            Some(Value::String(s)) => s == "true",
            _ => false,
        };

        if has_label {
            self.labeled.insert(name.clone());
        }

        if has_explicit_volumes {
            self.explicit_mount.insert(name);
        }

        Some(())
    }

    /// Build a compose override that bind-mounts the resolved providers directory at
    /// `/providers` inside each labeled service and remaps file-provider env vars to
    /// their container-side paths.
    fn local_file_providers_overlay(&self, providers_host_path: &Path) -> String {
        let mut labeled_services: Vec<&String> = self.labeled.iter().collect();
        labeled_services.sort();

        let volume = format!(
            "{}:{}",
            providers_host_path.display(),
            PROVIDERS_CONTAINER_PATH
        );

        let mut services_map = Mapping::new();

        for service in labeled_services.iter() {
            let mut service_override_config = Mapping::new();

            service_override_config.insert(
                Value::String("volumes".into()),
                Value::Sequence(vec![Value::String(volume.clone())]),
            );

            services_map.insert(
                Value::String(service.to_string()),
                Value::Mapping(service_override_config),
            );
        }

        let mut root = Mapping::new();
        root.insert(
            Value::String("services".into()),
            Value::Mapping(services_map),
        );

        serde_yaml::to_string(&Value::Mapping(root))
            .expect("yaml mapping should always convert to valid string")
    }
}

/// Collects docker compose `pull_policy` values per service so they can be translated into
/// the `kompose.image-pull-policy` label kompose reads when converting to Kubernetes manifests.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct PullPolicyServices {
    policies: BTreeMap<String, &'static str>,
}

impl PullPolicyServices {
    /// Scan the given compose file contents and build a kompose label overlay for any
    /// service-level `pull_policy` with a Kubernetes equivalent. Returns `None` if none exist.
    pub fn overlay_from_compose_files<'a>(
        compose_contents: impl Iterator<Item = &'a str>,
    ) -> Option<String> {
        let mut services = Self::default();
        for content in compose_contents {
            services.add_services_from(content);
        }

        if services.is_empty() {
            None
        } else {
            Some(services.kompose_label_overlay())
        }
    }

    /// Scan a compose file's YAML content, recording any service-level `pull_policy` that has
    /// a Kubernetes equivalent. Unsupported values are skipped with a warning.
    fn add_services_from(&mut self, yaml_content: &str) -> Option<()> {
        // Cheap pre-check to skip parsing and walking the tree for the common case where the
        // file has no `pull_policy` field at all.
        if !yaml_content.contains("pull_policy") {
            return Some(());
        }

        let value = serde_yaml::from_str::<Value>(yaml_content).ok()?;
        let services = value.get("services").and_then(|s| s.as_mapping())?;

        for (name, service) in services.into_iter() {
            self.add_service(name, service);
        }

        Some(())
    }

    fn add_service(&mut self, name: &Value, service: &Value) -> Option<()> {
        let name = name.as_str().map(str::to_string)?;
        let policy = service.get("pull_policy").and_then(|v| v.as_str())?;

        match kompose_pull_policy(policy) {
            Some(mapped) => {
                self.policies.insert(name, mapped);
            }
            None => warn!(
                "service '{name}' has pull_policy '{policy}' with no Kubernetes equivalent; skipping for kompose conversion"
            ),
        }

        Some(())
    }

    fn is_empty(&self) -> bool {
        self.policies.is_empty()
    }

    /// Build a compose overlay that labels each service so kompose sets a matching
    /// `imagePullPolicy` on its generated container.
    fn kompose_label_overlay(&self) -> String {
        let mut services_map = Mapping::new();

        for (service, policy) in &self.policies {
            let mut labels = Mapping::new();
            labels.insert(
                Value::String(KOMPOSE_IMAGE_PULL_POLICY_LABEL.into()),
                Value::String((*policy).into()),
            );

            let mut service_override = Mapping::new();
            service_override.insert(Value::String("labels".into()), Value::Mapping(labels));

            services_map.insert(
                Value::String(service.clone()),
                Value::Mapping(service_override),
            );
        }

        let mut root = Mapping::new();
        root.insert(
            Value::String("services".into()),
            Value::Mapping(services_map),
        );

        serde_yaml::to_string(&Value::Mapping(root))
            .expect("yaml mapping should always convert to valid string")
    }
}

/// Map a compose `pull_policy` value to the Kubernetes `imagePullPolicy` kompose's label expects.
///
/// Returns `None` for values with no Kubernetes equivalent (e.g. `build`, a build-vs-pull-time
/// concept that doesn't apply to a pre-built container image).
/// https://docs.docker.com/reference/compose-file/services/#pull_policy
fn kompose_pull_policy(policy: &str) -> Option<&'static str> {
    match policy {
        "always" => Some("Always"),
        "never" => Some("Never"),
        "if_not_present" | "missing" => Some("IfNotPresent"),
        "daily" | "weekly" => Some("Always"),
        p if p.contains("every") => Some("Always"),
        "build" => None,
        _ => None,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::{
        context::Context,
        formats::{
            OutputCollection, Sources,
            environment::{
                EnvironmentConfig, EnvironmentExecution,
                test_helpers::{docker_compose_env, register_compose_paths},
            },
            tests::{
                assert_check_errors, templatable_file_providers, template_context,
                variable_definitions,
            },
        },
        providers::{
            self,
            file::{
                DirFile, FileProvider, InlineDir, InlineFile, NamedFileProvider, RelativeFile,
                RequiredFile, SourceDir, StableSource, compose::NamedComposeFileProvider,
            },
            test_helpers::create_temp_dir_with_file,
        },
        templating::{Field, Scalar, Template},
    };
    use assert_fs::{fixture::PathChild, prelude::FileWriteStr};
    use indoc::indoc;
    use simple_test_case::test_case;
    use std::{collections::HashMap, path::PathBuf};

    // An example docker compose environment config to check parsing and templating
    const TEMPLATED_COMPOSE_ENVIRONMENT: &str = indoc!(
        r#"
        name: docker-compose-environment
        description: a templated docker compose environment
        variable_definitions:
          - name: foo
            description: a value foo
            allowed_values: ["foo1", "foo2"]
          - name: bar
            description: a value bar
            default: "bar"
        project_name: docker-compose-env
        env_vars:
            ENV_VAR: "value"
        compose_files:
          - name: foo.yaml
            kind: relative_path
            path: "{{ foo }}"
        file_providers:
          - name: bar.txt
            env_var: BAR
            kind: relative_path
            path: "{{ bar }}"
        "#
    );

    #[test]
    fn parse_compose_environment_success() {
        let config: EnvironmentConfig<EnvironmentExecution> =
            serde_yaml::from_str(TEMPLATED_COMPOSE_ENVIRONMENT)
                .expect("environment config to parse");

        let mut res = config.required_variables();
        res.sort(); // Sorting so variables are in a deterministic order for the assert_eq

        assert_eq!(res, &["bar", "foo"], "expected variables to match");
        assert_eq!(config.custom_providers.len(), 0);
    }

    #[test]
    fn try_template_docker_compose_succeeds() {
        let field_names = &["compose", "file", "env"];
        let ctx = template_context(field_names);

        let mut env_vars = HashMap::new();
        env_vars.insert("ENV_VAR".to_string(), Field::Pending("env".to_string()));

        let mut environment = EnvironmentConfig {
            variable_definitions: variable_definitions(field_names),
            execution: EnvironmentExecution::DockerCompose(DockerComposeEnvironment {
                resources: ComposeResources {
                    project_name: None,
                    compose_files: vec![NamedComposeFileProvider {
                        name: "compose.yaml".to_string(),
                        provider: ComposeFileProvider::RelativePath(RelativeFile {
                            path: Field::Pending("compose".to_string()),
                            src: None,
                        }),
                    }],
                },
                file_providers: templatable_file_providers(&["file"]),
                env_vars,
                output_collection: OutputCollection {
                    prometheus: Vec::new(),
                },
            }),
            ..EnvironmentConfig::empty()
        };

        let res = environment.try_template(&mut Vec::new(), &StableSource::Environment, &ctx);
        assert!(
            res.is_ok(),
            "expected to template successfully, got {res:?}"
        )
    }

    #[tokio::test]
    async fn docker_compose_environment_config_inline_succeeds() {
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

        let relative_compose_file = ComposeFileProvider::RelativePath(RelativeFile {
            path: Field::Resolved("file.txt".to_string()),
            src: Some(StableSource::Environment),
        });

        let relative_file = FileProvider::RelativePath(RelativeFile {
            path: Field::Resolved("file.txt".to_string()),
            src: Some(StableSource::Environment),
        });

        let mut environment = EnvironmentConfig {
            execution: EnvironmentExecution::DockerCompose(DockerComposeEnvironment {
                resources: ComposeResources {
                    project_name: None,
                    compose_files: vec![NamedComposeFileProvider {
                        name: "compose.yaml".to_string(),
                        provider: relative_compose_file,
                    }],
                },
                file_providers: vec![NamedFileProvider {
                    name: "file.txt".to_string(),
                    env_var: "FILE".to_string(),
                    provider: relative_file,
                }],
                env_vars: HashMap::new(),
                output_collection: OutputCollection {
                    prometheus: Vec::new(),
                },
            }),
            ..EnvironmentConfig::empty()
        };

        let result = environment
            .inline(&InlineMode::All, &ctx, &mut HashMap::new())
            .await;

        assert!(result.is_ok(), "Expected inline to succeed, got {result:?}");

        let expected_inline_compose_file = NamedComposeFileProvider {
            name: "compose.yaml".to_string(),
            provider: ComposeFileProvider::Inline(InlineFile {
                content: "example file content".to_string(),
            }),
        };
        let expected_inline_file = NamedFileProvider {
            name: "file.txt".to_string(),
            env_var: "FILE".to_string(),
            provider: FileProvider::Inline(InlineFile {
                content: "example file content".to_string(),
            }),
        };

        let compose = match &environment.execution {
            EnvironmentExecution::DockerCompose(compose) => compose,
            _ => panic!("Expected EnvironmentExecution::DockerCompose variant"),
        };

        assert_eq!(
            compose.resources.compose_files[0], expected_inline_compose_file,
            "Expected compose file to be inlined"
        );
        assert_eq!(
            compose.file_providers[0], expected_inline_file,
            "Expected file provider to be inlined"
        );
    }

    #[test]
    fn check_docker_compose_success() {
        let environment = EnvironmentConfig {
            execution: EnvironmentExecution::DockerCompose(DockerComposeEnvironment {
                resources: ComposeResources {
                    project_name: None,
                    compose_files: vec![NamedComposeFileProvider {
                        name: "compose.yaml".to_string(),
                        provider: ComposeFileProvider::Inline(InlineFile {
                            content: "content".to_string(),
                        }),
                    }],
                },
                file_providers: vec![NamedFileProvider {
                    name: "file.txt".to_string(),
                    env_var: "FILE".to_string(),
                    provider: FileProvider::Inline(InlineFile {
                        content: "content".to_string(),
                    }),
                }],
                env_vars: HashMap::new(),
                output_collection: OutputCollection {
                    prometheus: Vec::new(),
                },
            }),
            ..EnvironmentConfig::empty()
        };

        let ctx = Context::new();

        let res = environment.try_check(&mut Vec::new(), &ctx);
        assert!(res.is_ok(), "expected check to succeed, got {res:?}");
    }

    #[test]
    fn try_check_docker_compose_compose_file_errors() {
        let variables = &["foo"];
        let environment = EnvironmentConfig {
            variable_definitions: variable_definitions(variables),
            execution: EnvironmentExecution::DockerCompose(DockerComposeEnvironment {
                resources: ComposeResources {
                    project_name: None,
                    compose_files: vec![NamedComposeFileProvider {
                        name: "compose.yaml".to_string(),
                        provider: ComposeFileProvider::Required(RequiredFile {
                            message: "this is a required file".to_string(),
                        }),
                    }],
                },
                file_providers: Vec::new(),
                env_vars: HashMap::new(),
                output_collection: OutputCollection {
                    prometheus: Vec::new(),
                },
            }),
            ..EnvironmentConfig::empty()
        };

        let ctx = Context::new();

        let expected_err_kinds = &[checks::ErrorKind::RequiredFileMissing];

        assert_check_errors(environment, &ctx, expected_err_kinds);
    }

    #[test]
    fn try_check_docker_compose_file_provider_errors() {
        let variables = &["foo"];
        let environment = EnvironmentConfig {
            variable_definitions: variable_definitions(variables),
            execution: EnvironmentExecution::DockerCompose(DockerComposeEnvironment {
                resources: ComposeResources {
                    project_name: None,
                    compose_files: Vec::new(),
                },
                file_providers: vec![NamedFileProvider {
                    name: "file.txt".to_string(),
                    env_var: "FILE".to_string(),
                    provider: FileProvider::Required(RequiredFile {
                        message: "this is a required file".to_string(),
                    }),
                }],
                env_vars: HashMap::new(),
                output_collection: OutputCollection {
                    prometheus: Vec::new(),
                },
            }),
            ..EnvironmentConfig::empty()
        };

        let ctx = Context::new();

        let expected_err_kinds = &[checks::ErrorKind::RequiredFileMissing];

        assert_check_errors(environment, &ctx, expected_err_kinds);
    }

    #[test]
    fn docker_compose_setup_as_command_and_args_builds_correct_command() {
        let docker_compose = docker_compose_env(Some("test-project"), &["compose.yaml"]);

        let mut ctx = Context::new();
        let (_temp_dir, paths) = register_compose_paths(&mut ctx, &docker_compose);

        let result = docker_compose.setup_as_command_and_args("test-project", &[], &ctx);
        assert!(result.is_ok(), "Expected command to build successfully");

        let (cmd, args) = result.unwrap();
        assert_eq!(cmd, "docker");
        assert_eq!(
            args,
            vec![
                "compose",
                "-p",
                "test-project",
                "-f",
                &paths[0],
                "up",
                "-d",
                "--wait"
            ]
        );
    }

    #[test]
    fn docker_compose_setup_with_multiple_compose_files() {
        let docker_compose =
            docker_compose_env(Some("multi-compose"), &["base.yaml", "overlay.yaml"]);

        let mut ctx = Context::new();
        let (_temp_dir, paths) = register_compose_paths(&mut ctx, &docker_compose);

        let result = docker_compose.setup_as_command_and_args("multi-compose", &[], &ctx);
        assert!(result.is_ok(), "Expected command to build successfully");

        let (cmd, args) = result.unwrap();
        assert_eq!(cmd, "docker");
        assert_eq!(
            args,
            vec![
                "compose",
                "-p",
                "multi-compose",
                "-f",
                &paths[0],
                "-f",
                &paths[1],
                "up",
                "-d",
                "--wait"
            ]
        );
    }

    #[test]
    fn docker_compose_teardown_as_command_and_args_builds_correct_command() {
        let docker_compose = docker_compose_env(Some("test-project"), &["compose.yaml"]);

        let result = docker_compose.teardown_as_command_and_args("test-project");
        assert!(result.is_ok(), "Expected command to build successfully");

        let (cmd, args) = result.unwrap();
        assert_eq!(cmd, "docker");
        assert_eq!(args, vec!["compose", "-p", "test-project", "down"]);
    }

    #[test]
    fn docker_compose_command_fails_when_compose_file_path_unknown() {
        let docker_compose = docker_compose_env(Some("test-project"), &["compose.yaml"]);

        // Context without stored provider path
        let ctx = Context::new();

        let result = docker_compose.setup_as_command_and_args("test-project", &[], &ctx);
        assert!(
            result.is_err(),
            "Expected error when compose file path unknown"
        );

        let err = result.unwrap_err();
        assert!(
            matches!(&err, providers::Error::MissingProviderOutput { name } if name == "compose.yaml"),
            "Expected MissingProviderOutput error, got {err:?}"
        );
    }

    #[test]
    fn docker_compose_all_env_vars_includes_file_providers_and_explicit_vars() {
        let file_provider = FileProvider::Inline(InlineFile {
            content: "file content".to_string(),
        });

        let mut explicit_env_vars = HashMap::new();
        explicit_env_vars.insert(
            "EXPLICIT_VAR".to_string(),
            Field::Resolved(Scalar::from("explicit_value")),
        );

        let docker_compose = DockerComposeEnvironment {
            resources: ComposeResources {
                project_name: Some("test-project".to_string()),
                compose_files: Vec::new(),
            },
            file_providers: vec![NamedFileProvider {
                name: "config.txt".to_string(),
                env_var: "CONFIG_FILE".to_string(),
                provider: file_provider.clone(),
            }],
            env_vars: explicit_env_vars,
            output_collection: OutputCollection {
                prometheus: Vec::new(),
            },
        };

        let mut ctx = Context::new();
        ctx.store_provider_output_path(
            Provider::File { fp: &file_provider },
            PathBuf::from("/tmp/providers/config.txt"),
        );

        let out_dir = PathBuf::from("/tmp/output");
        let output_path = out_dir.join("rtf_output");

        let result = docker_compose.build_env_vars(&out_dir, &output_path, false, &ctx);
        assert!(result.is_ok(), "Expected env vars to be built successfully");

        let env_vars = result.unwrap();

        assert_eq!(
            env_vars.get("CONFIG_FILE"),
            Some(&"/tmp/providers/config.txt".to_string()),
        );
        assert_eq!(
            env_vars.get("EXPLICIT_VAR"),
            Some(&"explicit_value".to_string()),
        );
        assert_eq!(env_vars.get("OUTDIR"), Some(&"/tmp/output".to_string()));
        assert_eq!(
            env_vars.get("RTF_OUTPUT"),
            Some(&"/tmp/output/rtf_output".to_string()),
        );
    }

    #[test]
    fn docker_compose_uses_environment_name_when_project_name_not_set() {
        let docker_compose = docker_compose_env(None, &["compose.yaml"]);

        let mut ctx = Context::new();
        let _temp_dir = register_compose_paths(&mut ctx, &docker_compose);

        // Pass "fallback-name" as the environment name
        let result = docker_compose.setup_as_command_and_args("fallback-name", &[], &ctx);
        assert!(result.is_ok(), "Expected command to build successfully");

        let (_, args) = result.unwrap();
        // The project name should be "fallback-name" (passed as argument)
        assert_eq!(args[2], "fallback-name");
    }

    #[test]
    fn docker_compose_setup_with_inline_dir_multiple_files() {
        // Create a temp directory with compose files to simulate InlineDir output
        let (tmp, base_file) = create_temp_dir_with_file("base.yaml", "version: '3'");
        let overlay_file = tmp.child("overlay.yaml");
        overlay_file.write_str("version: '3'").unwrap();

        let inline_dir = InlineDir {
            files: vec![
                DirFile {
                    path: base_file.path().to_path_buf(),
                    content: "version: '3'".to_string(),
                },
                DirFile {
                    path: overlay_file.path().to_path_buf(),
                    content: "version: '3'".to_string(),
                },
            ],
        };

        let docker_compose = DockerComposeEnvironment {
            resources: ComposeResources {
                project_name: Some("inline-dir-test".to_string()),
                compose_files: vec![NamedComposeFileProvider {
                    name: "compose-dir".to_string(),
                    provider: ComposeFileProvider::InlineDir(inline_dir),
                }],
            },
            file_providers: Vec::new(),
            env_vars: HashMap::new(),
            output_collection: OutputCollection {
                prometheus: Vec::new(),
            },
        };

        // Register the directory path (not individual files)
        let mut ctx = Context::new();
        ctx.store_provider_output_path(
            Provider::ComposeFile {
                fp: &docker_compose.resources.compose_files[0].provider,
            },
            tmp.path().to_path_buf(),
        );

        let res = docker_compose.setup_as_command_and_args("inline-dir-test", &[], &ctx);
        assert!(res.is_ok(), "Expected command to build successfully");

        let (cmd, args) = res.unwrap();
        assert_eq!(cmd, "docker");

        // Verify structure: compose -p name -f file1 -f file2 up -d --wait
        let expected_args = vec![
            "compose".to_string(),
            "-p".to_string(),
            "inline-dir-test".to_string(),
            "-f".to_string(),
            base_file.to_str().unwrap().to_string(),
            "-f".to_string(),
            overlay_file.to_str().unwrap().to_string(),
            "up".to_string(),
            "-d".to_string(),
            "--wait".to_string(),
        ];
        assert_eq!(args, expected_args, "expected args to match");
    }

    #[test_case("already-valid", "already-valid"; "already valid name unchanged")]
    #[test_case("My Environment", "my-environment"; "uppercase and spaces")]
    #[test_case("foo.bar.baz", "foo-bar-baz"; "dots replaced with dashes")]
    #[test_case("--leading-dashes", "leading-dashes"; "leading dashes stripped")]
    #[test_case("__leading_underscores", "leading_underscores"; "leading underscores stripped")]
    #[test_case("MiXeD_CaSe-123", "mixed_case-123"; "mixed case lowered")]
    #[test_case("foo@bar!baz", "foo-bar-baz"; "special chars replaced")]
    #[test_case("123-starts-with-digit", "123-starts-with-digit"; "leading digit preserved")]
    #[test]
    fn slugify_compose_project_name_produces_valid_name(input: &str, expected: &str) {
        assert_eq!(slugify_compose_project_name(input), expected);
    }

    #[test]
    fn file_provider_services_categorises_correctly() {
        let yaml = indoc!(
            r#"
            services:
              true_bool_label:
                labels:
                  rtf.io/file-providers: true

              true_str_label:
                labels:
                  rtf.io/file-providers: "true"

              false_label:
                labels:
                  rtf.io/file-providers: false

              explicit_mount:
                volumes:
                  - foo:bar

              label_and_mount:
                labels:
                  rtf.io/file-providers: true
                volumes:
                  - foo:bar

              no_providers:
                image: nginx
            "#
        );

        let mut fps = FileProviderServices::default();
        fps.add_services_from(yaml);

        for s in ["true_bool_label", "true_str_label", "label_and_mount"] {
            assert!(fps.labeled.contains(s), "{s} should be marked as labeled");
        }

        for s in ["explicit_mount", "label_and_mount"] {
            assert!(
                fps.explicit_mount.contains(s),
                "{s} should be marked as having a mount"
            );
        }

        for s in ["false_label", "no_providers", "explicit_mount"] {
            assert!(
                !fps.labeled.contains(s),
                "{s} should not be marked as labeled"
            );
        }

        for s in [
            "true_bool_label",
            "true_str_label",
            "false_label",
            "no_providers",
        ] {
            assert!(
                !fps.explicit_mount.contains(s),
                "{s} should not be marked as having a mount"
            );
        }
    }

    #[test]
    fn file_provider_services_add_services_from_returns_none_for_invalid_yaml() {
        assert!(
            FileProviderServices::default()
                .add_services_from("{ not: valid: yaml: [")
                .is_none()
        );
    }

    #[test]
    fn generate_local_file_providers_overlay_adds_volume() {
        let fps = FileProviderServices {
            labeled: HashSet::from(["svc".to_string()]),
            ..Default::default()
        };
        let p = Path::new("/out/providers/setup_providers");
        let out = fps.local_file_providers_overlay(p);

        let expected = indoc!(
            r#"
            services:
              svc:
                volumes:
                - /out/providers/setup_providers:/providers
            "#
        );

        assert_eq!(out, expected);
    }

    #[test]
    fn generate_local_file_providers_overlay_adds_volume_for_multiple_services() {
        let fps = FileProviderServices {
            labeled: HashSet::from(["svc-a".to_string(), "svc-b".to_string()]),
            ..Default::default()
        };
        let p = Path::new("/out/providers/setup_providers");
        let out = fps.local_file_providers_overlay(p);

        let expected = indoc!(
            r#"
            services:
              svc-a:
                volumes:
                - /out/providers/setup_providers:/providers
              svc-b:
                volumes:
                - /out/providers/setup_providers:/providers
            "#
        );

        assert_eq!(out, expected);
    }

    #[test]
    fn build_env_vars_uses_host_paths_without_labeled_services() {
        let file_provider = FileProvider::Inline(InlineFile {
            content: "file content".to_string(),
        });

        let docker_compose = DockerComposeEnvironment {
            resources: ComposeResources {
                project_name: None,
                compose_files: Vec::new(),
            },
            file_providers: vec![NamedFileProvider {
                name: "config.txt".to_string(),
                env_var: "CONFIG_FILE".to_string(),
                provider: file_provider.clone(),
            }],
            env_vars: HashMap::new(),
            output_collection: OutputCollection {
                prometheus: Vec::new(),
            },
        };

        let mut ctx = Context::new();
        ctx.store_provider_output_path(
            Provider::File { fp: &file_provider },
            PathBuf::from("/tmp/providers/config.txt"),
        );

        let out_dir = PathBuf::from("/tmp/output");
        let output_path = out_dir.join("rtf_output");

        let result = docker_compose.build_env_vars(&out_dir, &output_path, false, &ctx);
        assert!(result.is_ok(), "Expected env vars to be built successfully");

        let env_vars = result.unwrap();
        assert_eq!(
            env_vars.get("CONFIG_FILE"),
            Some(&"/tmp/providers/config.txt".to_string()),
        );
    }

    #[test]
    fn build_env_vars_uses_container_paths_with_labeled_services() {
        let file_provider = FileProvider::Inline(InlineFile {
            content: "file content".to_string(),
        });

        let docker_compose = DockerComposeEnvironment {
            resources: ComposeResources {
                project_name: None,
                compose_files: Vec::new(),
            },
            file_providers: vec![NamedFileProvider {
                name: "config.txt".to_string(),
                env_var: "CONFIG_FILE".to_string(),
                provider: file_provider.clone(),
            }],
            env_vars: HashMap::new(),
            output_collection: OutputCollection {
                prometheus: Vec::new(),
            },
        };

        let mut ctx = Context::new();
        ctx.store_provider_output_path(
            Provider::File { fp: &file_provider },
            PathBuf::from("/tmp/providers/config.txt"),
        );

        let out_dir = PathBuf::from("/tmp/output");
        let output_path = out_dir.join("rtf_output");

        let result = docker_compose.build_env_vars(&out_dir, &output_path, true, &ctx);
        assert!(result.is_ok(), "Expected env vars to be built successfully");

        let env_vars = result.unwrap();
        assert_eq!(
            env_vars.get("CONFIG_FILE"),
            Some(&"/providers/config.txt".to_string()),
        );
    }

    #[test]
    fn build_env_vars_uses_container_paths_for_all_file_providers_when_labeled() {
        let file_provider_1 = FileProvider::Inline(InlineFile {
            content: "content 1".to_string(),
        });
        let file_provider_2 = FileProvider::Inline(InlineFile {
            content: "content 2".to_string(),
        });

        let docker_compose = DockerComposeEnvironment {
            resources: ComposeResources {
                project_name: None,
                compose_files: Vec::new(),
            },
            file_providers: vec![
                NamedFileProvider {
                    name: "config.txt".to_string(),
                    env_var: "CONFIG_FILE".to_string(),
                    provider: file_provider_1.clone(),
                },
                NamedFileProvider {
                    name: "secret.txt".to_string(),
                    env_var: "SECRET_FILE".to_string(),
                    provider: file_provider_2.clone(),
                },
            ],
            env_vars: HashMap::new(),
            output_collection: OutputCollection {
                prometheus: Vec::new(),
            },
        };

        let mut ctx = Context::new();
        ctx.store_provider_output_path(
            Provider::File {
                fp: &file_provider_1,
            },
            PathBuf::from("/tmp/providers/config.txt"),
        );
        ctx.store_provider_output_path(
            Provider::File {
                fp: &file_provider_2,
            },
            PathBuf::from("/tmp/providers/secret.txt"),
        );

        let out_dir = PathBuf::from("/tmp/output");
        let output_path = out_dir.join("rtf_output");

        let result = docker_compose.build_env_vars(&out_dir, &output_path, true, &ctx);
        assert!(result.is_ok(), "Expected env vars to be built successfully");

        let env_vars = result.unwrap();
        assert_eq!(
            env_vars.get("CONFIG_FILE"),
            Some(&"/providers/config.txt".to_string()),
        );
        assert_eq!(
            env_vars.get("SECRET_FILE"),
            Some(&"/providers/secret.txt".to_string()),
        );
    }

    #[test]
    fn pull_policy_overlay_maps_known_values_for_multiple_services() {
        let yaml = indoc!(
            r#"
            services:
              always-svc:
                pull_policy: always
              never-svc:
                pull_policy: never
              if-not-present-svc:
                pull_policy: if_not_present
              missing-svc:
                pull_policy: missing
              daily-svc:
                pull_policy: daily
              weekly-svc:
                pull_policy: weekly
              every-svc:
                pull_policy: every_6h
            "#
        );

        let mut services = PullPolicyServices::default();
        services.add_services_from(yaml);

        let expected = indoc!(
            r#"
            services:
              always-svc:
                labels:
                  kompose.image-pull-policy: Always
              daily-svc:
                labels:
                  kompose.image-pull-policy: Always
              every-svc:
                labels:
                  kompose.image-pull-policy: Always
              if-not-present-svc:
                labels:
                  kompose.image-pull-policy: IfNotPresent
              missing-svc:
                labels:
                  kompose.image-pull-policy: IfNotPresent
              never-svc:
                labels:
                  kompose.image-pull-policy: Never
              weekly-svc:
                labels:
                  kompose.image-pull-policy: Always
            "#
        );

        assert_eq!(services.kompose_label_overlay(), expected);
    }

    #[test]
    fn pull_policy_overlay_skips_unsupported_and_unset_policies() {
        let yaml = indoc!(
            r#"
            services:
              build-svc:
                pull_policy: build
              plain-svc:
                image: nginx
            "#
        );

        let mut services = PullPolicyServices::default();
        services.add_services_from(yaml);

        assert!(services.is_empty());
    }

    #[test]
    fn overlay_from_compose_files_returns_none_without_pull_policy() {
        let compose = "services:\n  web:\n    image: nginx\n";

        assert!(PullPolicyServices::overlay_from_compose_files(std::iter::once(compose)).is_none());
    }

    #[test]
    fn overlay_from_compose_files_merges_across_multiple_compose_files() {
        let base = "services:\n  web:\n    pull_policy: always\n";
        let extra = "services:\n  worker:\n    pull_policy: never\n";

        let overlay = PullPolicyServices::overlay_from_compose_files([base, extra].into_iter())
            .expect("overlay expected");

        let expected = indoc!(
            r#"
            services:
              web:
                labels:
                  kompose.image-pull-policy: Always
              worker:
                labels:
                  kompose.image-pull-policy: Never
            "#
        );

        assert_eq!(overlay, expected);
    }

    #[test]
    fn build_env_vars_error_when_file_provider_path_unknown() {
        let file_provider = FileProvider::Inline(InlineFile {
            content: "file content".to_string(),
        });

        let docker_compose = DockerComposeEnvironment {
            resources: ComposeResources {
                project_name: None,
                compose_files: Vec::new(),
            },
            file_providers: vec![NamedFileProvider {
                name: "config.txt".to_string(),
                env_var: "CONFIG_FILE".to_string(),
                provider: file_provider,
            }],
            env_vars: HashMap::new(),
            output_collection: OutputCollection {
                prometheus: Vec::new(),
            },
        };

        let ctx = Context::new();
        let out_dir = PathBuf::from("output_dir");
        let output_path = out_dir.join("path");

        let result = docker_compose.build_env_vars(&out_dir, &output_path, false, &ctx);
        assert!(
            result.is_err(),
            "Expected error when file provider path is not registered"
        );

        let err = result.unwrap_err();
        assert!(
            matches!(&err, providers::Error::MissingProviderOutput { name } if name == "config.txt"),
            "Expected MissingProviderOutput error, got {err:?}"
        );
    }
}
