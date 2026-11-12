use crate::{
    checks::{self, Check, CheckArrayDuplicates, DedupArray},
    context::{PathKind, ResolutionContext},
    formats::OutputCollection,
    inlining::{self, Inline, InlineMode, InlinedProvider, ProviderCacheKey},
    providers::{
        self,
        file::{NamedFileProvider, StableSource, manifest::NamedManifestFileProvider},
    },
    run::{OUTDIR, OUTPUT_PATH, Provider, RunProviders, ValidateEnvironment},
    templating::{self, Field, Scalar, Template, TemplateContext},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs::read_dir,
    path::{Path, PathBuf},
    pin::Pin,
};

const PROVIDERS_CONTAINER_PATH: &str = "/providers";

pub trait NamedManifestFiles {
    fn manifest_files(&self) -> &Vec<NamedManifestFileProvider>;
    fn manifest_files_mut(&mut self) -> &mut Vec<NamedManifestFileProvider>;
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct ManifestEnvironment<M: ValidateEnvironment> {
    #[serde(flatten)]
    pub resources: M,
    /// A list of all other files this environment depends on
    #[serde(default)]
    pub file_providers: Vec<NamedFileProvider>,
    // Environment variables to set
    #[serde(default)]
    pub env_vars: HashMap<String, Field<Scalar>>,
    #[serde(default)]
    pub output_collection: OutputCollection,
}

impl<M: ValidateEnvironment> ManifestEnvironment<M> {
    pub fn build_env_vars(
        &self,
        out_dir: &Path,
        output_path: &Path,
        has_labeled_services: bool,
        ctx: &impl ResolutionContext,
    ) -> providers::Result<HashMap<String, String>> {
        let mut vars = self.explicit_env_vars();

        vars.extend(self.file_provider_env_vars(has_labeled_services, ctx)?);
        vars.insert(OUTDIR.to_string(), out_dir.display().to_string());
        vars.insert(OUTPUT_PATH.to_string(), output_path.display().to_string());

        Ok(vars)
    }

    fn explicit_env_vars(&self) -> HashMap<String, String> {
        self.env_vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.as_resolved().to_string()))
            .collect()
    }

    fn file_provider_env_vars(
        &self,
        has_labeled_services: bool,
        ctx: &impl ResolutionContext,
    ) -> providers::Result<HashMap<String, String>> {
        let mut vars = HashMap::new();

        for nfp in self.file_providers.iter() {
            let path = ctx
                .known_provider_output_path(Provider::File { fp: &nfp.provider })
                .ok_or(providers::Error::MissingProviderOutput {
                    name: nfp.name.clone(),
                })?;

            // When labeled services are present, the overlay bind-mounts the resolved providers_dir
            // at PROVIDERS_CONTAINER_PATH inside the container. Remap file provider env vars from
            // their host paths (returned by all_env_vars) to the container paths so that ${VAR}
            // references in manifest files expand to the correct in-container location.
            //
            // We only rewrite the environment variables if at least one service has the label present
            // This will break any services that try to volume mount to the files on the local filesystem
            // but we are ok with this since the presence of a label implies the user wants this to work
            // on the Orchestrator and any services attempting to mount to the local filesystem will fail
            if has_labeled_services {
                vars.insert(
                    nfp.env_var.clone(),
                    format!("{}/{}", PROVIDERS_CONTAINER_PATH, nfp.name),
                );
            } else {
                vars.insert(nfp.env_var.clone(), path.to_string_lossy().to_string());
            }
        }

        Ok(vars)
    }
}

impl<M: ValidateEnvironment + NamedManifestFiles> ManifestEnvironment<M> {
    /// Collect all manifest file paths from the resolved providers.
    ///
    /// Handles both single files and directories of manifest files. When a provider
    /// outputs a directory, all .yaml/.yml files within it are collected and sorted.
    pub fn manifest_file_paths(
        &self,
        ctx: &impl ResolutionContext,
    ) -> providers::Result<Vec<PathBuf>> {
        let mut paths = Vec::new();

        for ncfp in self.resources.manifest_files().iter() {
            let path = ctx
                .known_provider_output_path(Provider::ComposeFile { fp: &ncfp.provider })
                .ok_or(providers::Error::MissingProviderOutput {
                    name: ncfp.name.clone(),
                })?;

            match ctx.path_kind(&path) {
                PathKind::File => {
                    paths.push(path);
                }
                PathKind::OccupiedDir => {
                    paths.extend(collect_manifest_files(&path)?);
                }
                _ => {
                    return Err(providers::Error::ProviderOutputNotFileOrDir {
                        path_kind: ctx.path_kind(path),
                    });
                }
            }
        }

        Ok(paths)
    }

    /// Summarise each manifest provider without reading its content, in provider order.
    ///
    /// Safe to call on a partially templated environment: providers that still depend on
    /// variables without a value are reported as [ManifestSummary::Unresolved].
    pub fn manifest_summaries(&self, ctx: &impl ResolutionContext) -> Vec<ManifestSummary> {
        self.resources
            .manifest_files()
            .iter()
            .map(|ncfp| {
                let required_variables = ncfp.required_variables();
                if required_variables.is_empty() {
                    return ManifestSummary::Resolved {
                        name: ncfp.name.clone(),
                        url: ncfp.provider.github_permalink(ctx),
                    };
                }

                ManifestSummary::Unresolved {
                    name: ncfp.name.clone(),
                    required_variables,
                }
            })
            .collect()
    }

    pub async fn inline_manifests<'a>(
        &'a mut self,
        ctx: &'a impl ResolutionContext,
        cache: &'a mut HashMap<ProviderCacheKey, InlinedProvider>,
    ) -> inlining::Result<()> {
        let mut errs = inlining::ErrorBuilder::new();

        errs.append(
            self.resources
                .manifest_files_mut()
                .try_inline(InlineMode::All, ctx, cache)
                .await,
        );

        errs.into_result(())
    }
}

impl<M: ValidateEnvironment> RunProviders for ManifestEnvironment<M> {
    fn named_providers<'a>(&'a self) -> Vec<(&'a str, Provider<'a>)> {
        let mut providers = self.resources.named_providers();
        providers.extend(self.file_providers.named_providers());

        providers
    }
}

impl<M: ValidateEnvironment> Inline for ManifestEnvironment<M> {
    fn try_inline<'a>(
        &'a mut self,
        mode: InlineMode,
        ctx: &'a impl ResolutionContext,
        cache: &'a mut HashMap<ProviderCacheKey, InlinedProvider>,
    ) -> Pin<Box<dyn Future<Output = inlining::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let mut errs = inlining::ErrorBuilder::new();

            errs.append(self.resources.try_inline(mode, ctx, cache).await);
            errs.append(self.file_providers.try_inline(mode, ctx, cache).await);

            errs.into_result(())
        })
    }
}

impl<M: ValidateEnvironment> Check for ManifestEnvironment<M> {
    fn try_check(
        &self,
        path: &mut Vec<String>,
        ctx: &impl ResolutionContext,
    ) -> checks::Result<()> {
        let mut errs = checks::ErrorBuilder::new();

        errs.append(self.resources.try_check(path, ctx));
        for nfp in self.file_providers.iter() {
            errs.append(nfp.try_check_nested(path, "file_providers", ctx));
        }
        errs.append(self.output_collection.try_check(path, ctx));

        errs.into_result(())
    }
}

impl<M: ValidateEnvironment> CheckArrayDuplicates for ManifestEnvironment<M> {
    const BASE_PATH: &str = M::BASE_PATH;

    fn deduplicated_arrays<'a>(&'a mut self) -> Vec<(&'static str, DedupArray<'a>)> {
        let mut arrays = self.resources.deduplicated_arrays();
        arrays.push(("file_providers", DedupArray::Nfp(&mut self.file_providers)));
        arrays.extend(self.output_collection.deduplicated_arrays());

        arrays
    }
}

impl<M: ValidateEnvironment> Template for ManifestEnvironment<M> {
    fn required_variables(&self) -> Vec<String> {
        let mut vals = self.resources.required_variables();
        vals.extend(self.file_providers.required_variables());
        vals.extend(self.env_vars.required_variables());

        vals
    }

    fn validate_context(
        &self,
        path: &mut Vec<String>,
        allowed_variables: &HashSet<&String>,
        file_source: &StableSource,
        ctx: &TemplateContext,
    ) -> templating::Result<()> {
        let mut errs = templating::ErrorBuilder::from(self.resources.validate_context(
            path,
            allowed_variables,
            file_source,
            ctx,
        ));
        errs.append(self.file_providers.validate_context_nested(
            path,
            "file_providers",
            allowed_variables,
            file_source,
            ctx,
        ));
        errs.append(self.env_vars.validate_context_nested(
            path,
            "env_vars",
            allowed_variables,
            file_source,
            ctx,
        ));

        errs.into_result(())
    }

    fn try_template(
        &mut self,
        path: &mut Vec<String>,
        file_source: &StableSource,
        ctx: &TemplateContext,
    ) -> templating::Result<()> {
        let mut errs =
            templating::ErrorBuilder::from(self.resources.try_template(path, file_source, ctx));
        errs.append(self.file_providers.try_template_nested(
            path,
            "file_providers",
            file_source,
            ctx,
        ));
        errs.append(
            self.env_vars
                .try_template_nested(path, "env_vars", file_source, ctx),
        );

        errs.into_result(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestSummary {
    Resolved {
        name: String,
        url: Option<String>,
    },
    Unresolved {
        name: String,
        required_variables: Vec<String>,
    },
}

/// Collect all YAML manifest files from a directory.
fn collect_manifest_files(dir: &Path) -> providers::Result<Vec<PathBuf>> {
    let mut files = Vec::new();

    for entry in read_dir(dir)? {
        let path = entry?.path();
        if path.is_file()
            && path
                .extension()
                .is_some_and(|ext| ext == "yaml" || ext == "yml")
        {
            files.push(path);
        }
    }

    files.sort();

    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        context::Context,
        formats::environment::{docker_compose::DockerComposeEnvironment, k8s::K8sEnvironment},
    };
    use indoc::indoc;

    const GITHUB_URL: &str = "https://github.com/my-org/my-repo/blob/main/base.yaml";

    fn resolved(name: &str, url: Option<&str>) -> ManifestSummary {
        ManifestSummary::Resolved {
            name: name.to_string(),
            url: url.map(String::from),
        }
    }

    fn unresolved(name: &str, variables: &[&str]) -> ManifestSummary {
        ManifestSummary::Unresolved {
            name: name.to_string(),
            required_variables: variables.iter().map(|v| v.to_string()).collect(),
        }
    }

    #[test]
    fn manifest_summaries_k8s_reports_pending_providers_as_unresolved() {
        let env: K8sEnvironment = yaml_serde::from_str(indoc!(
            r#"
            resources:
              - name: base.yaml
                kind: github_file
                org: my-org
                repo: my-repo
                path: base.yaml
                git_ref: main
              - name: overlay.yaml
                kind: github_file
                org: my-org
                repo: my-repo
                path: "{{ overlay_path }}"
                git_ref: "{{ overlay_ref }}"
              - name: inline.yaml
                kind: inline
                content: "kind: ConfigMap"
            "#
        ))
        .unwrap();

        assert_eq!(
            env.manifest_summaries(&Context::new()),
            vec![
                resolved("base.yaml", Some(GITHUB_URL)),
                unresolved("overlay.yaml", &["overlay_path", "overlay_ref"]),
                resolved("inline.yaml", None),
            ]
        );
    }

    #[test]
    fn manifest_summaries_docker_compose_reports_pending_providers_as_unresolved() {
        let env: DockerComposeEnvironment = yaml_serde::from_str(indoc!(
            r#"
            compose_files:
              - name: base.yaml
                kind: github_file
                org: my-org
                repo: my-repo
                path: base.yaml
                git_ref: main
              - name: overlay.yaml
                kind: relative_path
                path: "{{ overlay_path }}"
            "#
        ))
        .unwrap();

        assert_eq!(
            env.manifest_summaries(&Context::new()),
            vec![
                resolved("base.yaml", Some(GITHUB_URL)),
                unresolved("overlay.yaml", &["overlay_path"]),
            ]
        );
    }
}
