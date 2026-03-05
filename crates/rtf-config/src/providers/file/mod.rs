//! The core [FileProvider] trait and currently supported file provider implementations.
use crate::{
    checks::{self, Check},
    context::{PathKind, ResolutionContext},
    enum_impl_check,
    inlining::{self, InlineMode},
    providers,
    run::{ExtractRelativeFiles, RunProviders, try_read_relative_dir, try_read_relative_file},
    templating::{self, Field, Template, TemplateContext},
};
use rtf_derive::Template;
use rtf_integrations::github::Client;
use schemars::{JsonSchema, generate::SchemaSettings};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    collections::{HashMap, HashSet},
    fmt, io,
    ops::{Deref, DerefMut},
    path::{Component, Path, PathBuf},
    str::FromStr,
};

pub mod apollo;
pub mod compose;
pub mod custom;
pub mod github;
mod source;
pub mod utility;

pub use source::{RawSource, SourceDir};

/// Something that can obtain or synthesise utf-8 file content based on a user provided
/// specification.
///
/// This trait is deliberately pub(crate) rather than pub so that the validation and resolution
/// logic is only exposed through the public API as part of the methods on the config file structs.
#[allow(async_fn_in_trait)]
pub(crate) trait AsUtf8FileContent:
    Check + Serialize + DeserializeOwned + fmt::Debug
{
    /// Attempt to convert this file provider into an InlineFile
    async fn try_into_inline_file(
        &self,
        ctx: &impl ResolutionContext,
    ) -> inlining::Result<InlineFile> {
        let content = self.try_get_file_content(ctx).await?;

        Ok(InlineFile { content })
    }

    /// Attempt to run this file provider and convert it into the required file content.
    async fn try_get_file_content(&self, ctx: &impl ResolutionContext)
    -> providers::Result<String>;
}

/// Helper macro for stamping out implementations of the AsUtf8FileContent trait on an enum where
/// each variant is a wrapper around a type that already implements the trait.
#[macro_export]
macro_rules! enum_impl_as_utf8_file_content {
    ($enum:ident => $($variant:ident),+) => {
        impl AsUtf8FileContent for $enum {
            async fn try_get_file_content(
                &self,
                ctx: &impl ResolutionContext,
            ) -> providers::Result<String> {
                match self {
                    $(Self::$variant(inner) => inner.try_get_file_content(ctx).await,)+
                }
            }
        }
    };
}

impl<T> ResolveFileContent for T
where
    T: AsUtf8FileContent,
{
    async fn try_get_all_file_contents(
        &self,
        target: impl AsRef<Path>,
        ctx: &impl ResolutionContext,
    ) -> providers::Result<Vec<DirFile>> {
        Ok(vec![DirFile {
            path: target.as_ref().to_path_buf(),
            content: self.try_get_file_content(ctx).await?,
        }])
    }
}

/// Something that can obtain or synthesise the contents of multiple utf-8 files based on a user
/// provided specification.
#[allow(async_fn_in_trait)]
pub(crate) trait ResolveFileContent:
    Check + Serialize + DeserializeOwned + fmt::Debug
{
    async fn try_get_all_file_contents(
        &self,
        target: impl AsRef<Path>,
        ctx: &impl ResolutionContext,
    ) -> providers::Result<Vec<DirFile>>;

    /// Attempt to convert this file provider into an InlineDir
    async fn try_into_inline_files(
        &self,
        ctx: &impl ResolutionContext,
    ) -> inlining::Result<InlineDir> {
        let files = self.try_get_all_file_contents(PathBuf::new(), ctx).await?;

        Ok(InlineDir { files })
    }
}

impl<T> ResolveAndWrite for T
where
    T: ResolveFileContent,
{
    async fn resolve_and_write(
        &self,
        target: impl AsRef<Path>,
        ctx: &mut impl ResolutionContext,
    ) -> providers::Result<()> {
        let files = self.try_get_all_file_contents(target, ctx).await?;
        for file in files.into_iter() {
            if let Some(parent) = file.path.parent() {
                ctx.create_dir_all(parent)?;
            }
            ctx.write(file.path, file.content)?;
        }

        Ok(())
    }
}

/// Logic for running a file provider and writing its output to the target [Path].
///
/// Most [FileProvider] implementations can safely ignore providing a custom implementation for
/// this trait if all they need to do is write out a single file, and instead just implement
/// [AsUtf8FileContent] or [ResolveFileContent] which will give a default implementation of this
/// trait. If however you need to write out multiple files or run some additional logic after
/// writing out a file (such as making it executable) then you should implement this trait
/// directly.
#[allow(async_fn_in_trait)]
pub(crate) trait ResolveAndWrite: Check + Serialize + DeserializeOwned + fmt::Debug {
    async fn resolve_and_write(
        &self,
        target: impl AsRef<Path>,
        ctx: &mut impl ResolutionContext,
    ) -> providers::Result<()>;
}

/// Helper macro for stamping out implementations of the ResolveAndWrite trait on an enum where
/// each variant is a wrapper around a type that already implements the trait.
#[macro_export]
macro_rules! enum_impl_resolve_and_write {
    ($enum:ident => $($variant:ident),+) => {
        impl ResolveAndWrite for $enum {
            async fn resolve_and_write(
                &self,
                target: impl AsRef<Path>,
                ctx: &mut impl ResolutionContext,
            ) -> $crate::providers::Result<()> {
                match self {
                    $(Self::$variant(inner) => inner.resolve_and_write(target, ctx).await,)+
                }
            }
        }
    };
}

/// Helper macro for implementing FileProvider::inline
/// The special cases for this macro are statically defined in the macro.
/// All the FileProviders that implement IntoUtf8Content have to be specified in the macro arguments
macro_rules! impl_inline {
    (
        $self:expr, $mode:expr, $ctx:expr;
        $($inline:ident),*
    ) => {
        match (&mut *$self, $mode) {
            // FromCommand just inlines any file providers it wraps using the mode provided
            (Self::FromCommand(inner), mode) => {
                inner.inline(mode, $ctx).await?;
                Ok(())
            }

            // MergeYaml is a special case, when inlining relative files only the inner providers
            // should be inlined
            (Self::MergeYaml(inner), mode) => {
                match mode {
                    InlineMode::All => {
                        *$self = FileProvider::Inline(inner.try_into_inline_file($ctx).await?);
                        Ok(())
                    }
                    InlineMode::RelativeFiles => {
                        inner.inline_all_relative_paths($ctx).await?;
                        Ok(())
                    }
                }
            }

            // RelativePaths get inlined the same way in both modes
            (FileProvider::RelativeDir(inner), _) => {
                *$self = FileProvider::InlineDir(inner.try_into_inline_files($ctx).await?);
                Ok(())
            }
            (FileProvider::RelativePath(inner), _) => {
                *$self = FileProvider::Inline(inner.try_into_inline_file($ctx).await?);
                Ok(())
            }

            // No other file providers need to do anything when mode is RelativeFiles
            (_, InlineMode::RelativeFiles) => Ok(()),

            // The methods required for inlining the rest of the providers
            (Self::Inline(_) | Self::InlineDir(_), InlineMode::All) => Ok(()),
            (Self::GraphosSubgraphs(inner), InlineMode::All) => {
                *$self = FileProvider::InlineDir(inner.inline($ctx).await?);
                Ok(())
            }
            $((Self::$inline(inner), InlineMode::All) => {
                *$self = FileProvider::Inline(inner.try_into_inline_file($ctx).await?);
                Ok(())
            })*
        }
    };
}

/// # Named File Provider
///
/// Shared metadata that wraps every file provider.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct NamedFileProvider {
    /// The name to use for the output produced by this provider
    ///
    /// This can be either a file or a directory depending on the file provider.
    pub name: String,
    /// The environment variable to place the absolute path to this providers output
    pub env_var: String,
    #[serde(flatten)]
    pub provider: FileProvider,
}

impl Deref for NamedFileProvider {
    type Target = FileProvider;

    fn deref(&self) -> &Self::Target {
        &self.provider
    }
}

impl DerefMut for NamedFileProvider {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.provider
    }
}

// Adding this custom implementation so that file providers and the environment variable
// used to identify the file provider are added to the path when templating a
// NamedFileProvider
impl Template for NamedFileProvider {
    fn required_variables(&self) -> Vec<String> {
        self.provider.required_variables()
    }

    fn validate_context(
        &self,
        path: &mut Vec<String>,
        allowed_variables: &HashSet<&String>,
        file_source: &SourceDir,
        ctx: &TemplateContext,
    ) -> templating::Result<()> {
        let tail = self.env_var.clone();
        self.provider
            .validate_context_nested(path, &tail, allowed_variables, file_source, ctx)
    }

    fn try_template(
        &mut self,
        path: &mut Vec<String>,
        file_source: &SourceDir,
        ctx: &TemplateContext,
    ) -> templating::Result<()> {
        let tail = self.env_var.clone();

        match &mut self.provider {
            FileProvider::Conditional(c) => {
                self.provider = c.try_collapse(path, ctx)?;
                self.provider
                    .try_template_nested(path, &tail, file_source, ctx)?
            }

            FileProvider::CustomProvider(cp) => {
                let from_command = cp.expand_and_template(path, file_source, ctx)?;
                self.provider = FileProvider::FromCommand(from_command);
            }

            _ => self
                .provider
                .try_template_nested(path, &tail, file_source, ctx)?,
        }

        Ok(())
    }
}

impl Check for NamedFileProvider {
    fn try_check(
        &self,
        path: &mut Vec<String>,
        ctx: &impl ResolutionContext,
    ) -> checks::Result<()> {
        let mut errs = checks::ErrorBuilder::new();
        let mut err_path = path.clone();
        err_path.push(self.env_var.clone());

        let name_relative_path = match PathBuf::from_str(&self.name) {
            Ok(p) => p,
            Err(_) => {
                errs.push(
                    checks::ErrorKind::InvalidRelativePath,
                    "file provider name is not a valid path",
                    &err_path,
                );
                PathBuf::new()
            }
        };
        errs.append(check_relative_path_specifiers(
            &name_relative_path,
            &mut err_path,
        ));

        let tail = self.env_var.clone();
        errs.append(self.provider.try_check_nested(path, tail, ctx));

        errs.into_result(())
    }
}

/// Helper function for checking for invalid relative path specifiers
pub(crate) fn check_relative_path_specifiers(
    relative_path: &Path,
    path: &mut [String],
) -> checks::Result<()> {
    if !relative_path
        .components()
        .all(|c| matches!(c, Component::Normal(_)))
    {
        return Err(checks::Errors::new(
            checks::ErrorKind::InvalidPathSpecifiers,
            "relative paths are not allowed to use \".\" or \"..\" notation or start with a leading \"/\"",
            path,
        ));
    }

    Ok(())
}

/// # File Provider
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, Template)]
#[serde(rename_all = "snake_case", tag = "kind")]
// If you are adding a FileProvider please make sure to also add a parse test to
// the all_fields_templated test in this file
pub enum FileProvider {
    BuildRouterFromSource(apollo::BuildRouterFromSource),
    Conditional(utility::Conditional),
    CustomProvider(custom::CustomProvider),
    FromCommand(utility::FromCommand),
    GithubFile(github::GithubFile),
    GraphosCannedOps(apollo::GraphosCannedOps),
    GraphosCannedOpsById(apollo::GraphosCannedOpsById),
    GraphosSubgraphRouterUrlOverrides(apollo::GraphosSubgraphRouterUrlOverrides),
    GraphosSubgraphs(apollo::GraphosSubgraphs),
    GraphosSubgraphNames(apollo::GraphosSubgraphNames),
    GraphosSupergraph(apollo::GraphosSupergraph),
    Inline(InlineFile),
    InlineDir(InlineDir),
    MergeYaml(utility::MergeYaml),
    OfflineGraphosLicense(apollo::OfflineGraphosLicense),
    RelativeDir(RelativeDir),
    RelativePath(RelativeFile),
    Required(RequiredFile),
    RouterDownloadScript(apollo::RouterDownloadScript),
    Templated(utility::TemplatedFile),
}

impl FileProvider {
    pub fn json_schema() -> serde_json::Value {
        let mut settings = SchemaSettings::default();
        settings.inline_subschemas = true;
        let generator = settings.into_generator();
        let schema = generator.into_root_schema_for::<Self>();

        schema.to_value()
    }

    /// Recursively inline file providers into their inline form
    pub async fn inline(
        &mut self,
        mode: &InlineMode,
        ctx: &impl ResolutionContext,
    ) -> inlining::Result<()> {
        impl_inline!(
            self, mode, ctx;
            BuildRouterFromSource,
            Conditional,
            CustomProvider,
            GithubFile,
            GraphosCannedOps,
            GraphosCannedOpsById,
            GraphosSubgraphRouterUrlOverrides,
            GraphosSubgraphNames,
            GraphosSupergraph,
            OfflineGraphosLicense,
            Required,
            RouterDownloadScript,
            Templated
        )
    }
}

impl ExtractRelativeFiles for FileProvider {
    async fn try_extract_relative_files(
        &self,
        files: &mut HashMap<PathBuf, String>,
        ctx: &impl ResolutionContext,
    ) -> providers::Result<()> {
        match self {
            Self::RelativePath(p) => try_read_relative_file(p, files, ctx).await,
            Self::RelativeDir(p) => try_read_relative_dir(p, files, ctx).await,
            Self::FromCommand(p) => Box::pin(p.inner.try_extract_relative_files(files, ctx)).await,
            Self::MergeYaml(p) => p.try_extract_relative_files(files, ctx).await,

            // We deliberately list out every variant here so we are forced to think about whether
            // or not new variants have internal relative files that we need to resolve.
            Self::BuildRouterFromSource(_)
            | Self::Conditional(_)
            | Self::CustomProvider(_)
            | Self::GithubFile(_)
            | Self::GraphosCannedOps(_)
            | Self::GraphosCannedOpsById(_)
            | Self::GraphosSubgraphRouterUrlOverrides(_)
            | Self::GraphosSubgraphs(_)
            | Self::GraphosSubgraphNames(_)
            | Self::GraphosSupergraph(_)
            | Self::Inline(_)
            | Self::InlineDir(_)
            | Self::OfflineGraphosLicense(_)
            | Self::Required(_)
            | Self::RouterDownloadScript(_)
            | Self::Templated(_) => Ok(()),
        }
    }
}

// Each time we add a new variant to the FileProvider enum above we need to remember to add it to
// the macro invocation below in order to update the trait implementations for the enum. (You can't
// really forget to do this as the compiler will complain about missing match arms if you do!)
macro_rules! enum_impl_file_provider {
    ($($variant:ident,)+) => {
        enum_impl_check!(FileProvider => $($variant),+);
        enum_impl_resolve_and_write!(FileProvider => $($variant),+);
    };
}

enum_impl_file_provider!(
    BuildRouterFromSource,
    Conditional,
    CustomProvider,
    FromCommand,
    GithubFile,
    GraphosCannedOps,
    GraphosCannedOpsById,
    GraphosSubgraphRouterUrlOverrides,
    GraphosSubgraphs,
    GraphosSubgraphNames,
    GraphosSupergraph,
    Inline,
    InlineDir,
    MergeYaml,
    OfflineGraphosLicense,
    RelativeDir,
    RelativePath,
    Required,
    RouterDownloadScript,
    Templated,
);

/// # Inline file
///
/// The simplest form of file provider: the user specifies the contents of the file inline within
/// their config file.
///
/// ```yaml
/// - name: "my-file.txt"
///   env_var: MY_FILE
///   kind: inline
///   content: |
///     my raw file content.
///     specified inline within an RTF config file.
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema, Template)]
pub struct InlineFile {
    /// The text to write out as the contents of the generated file.
    #[template(skip)]
    pub(crate) content: String,
}

impl AsUtf8FileContent for InlineFile {
    async fn try_get_file_content(
        &self,
        _ctx: &impl ResolutionContext,
    ) -> providers::Result<String> {
        Ok(self.content.clone())
    }
}

impl Check for InlineFile {
    fn try_check(
        &self,
        _path: &mut Vec<String>,
        _ctx: &impl ResolutionContext,
    ) -> checks::Result<()> {
        Ok(())
    }
}

/// # Inline directory
///
/// An inline representation of a directory of files. The environment variable will be set to the path
/// of the directory itself. All files within that directory will need to be referenced using a combination
/// of this environment variable and its `path`.
///
/// This file provider primarily exists so that other file providers that produce a directory of files
/// can be converted into their inline representations.
///
/// If, as a user of RTF, you need to specify multiple inline files, we *strongly* advise you use an
/// `inline` file provider for each file and that you DO NOT use this file provider.
///
/// ```yaml
/// - name: "my-directory"
///   env_var: MY_DIRECTORY
///   kind: inline_dir
///   files:
///     - path: file1.txt
///       content: |
///         content for file1
///     - path: nested/file2.txt
///       content: |
///         content for file2
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema, Template)]
pub struct InlineDir {
    /// A list of inline files stored in the directory
    #[template(skip)]
    pub(crate) files: Vec<DirFile>,
}

impl ResolveAndWrite for InlineDir {
    async fn resolve_and_write(
        &self,
        target: impl AsRef<Path>,
        ctx: &mut impl ResolutionContext,
    ) -> providers::Result<()> {
        for file in self.files.iter() {
            let abs_path = target.as_ref().join(&file.path);
            if let Some(parent) = abs_path.parent() {
                // Using create_dir_all here ensures that no matter how deeply
                // nested the path is within the directory, it gets created.
                // If it already exists then this is a no op
                ctx.create_dir_all(parent)?;
                ctx.write(&abs_path, &file.content)?;
            }
        }

        Ok(())
    }
}

impl Check for InlineDir {
    fn try_check(
        &self,
        path: &mut Vec<String>,
        _ctx: &impl ResolutionContext,
    ) -> checks::Result<()> {
        let mut errs = checks::ErrorBuilder::new();

        for file in self.files.iter() {
            errs.append(check_relative_path_specifiers(&file.path, path));
        }

        errs.into_result(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
pub struct DirFile {
    /// The relative path to the generated file within the directory
    pub path: PathBuf,

    /// The text to write out as the contents of the generated file.
    pub content: String,
}

/// # Relative path
///
/// A relative path from the containing config file to a target file that should be made available
/// as part of the test run. This provider works both with local files and files within GitHub
/// if the containing config file was pulled from a repository.
///
/// ```yaml
/// - name: "my-file.txt"
///   env_var: MY_FILE
///   kind: relative_path
///   path: "../../resources/test-data/my-file.txt"
/// ```
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct RelativeFile {
    /// The relative path from the containing config file to the target file.
    pub(crate) path: Field<String>,

    /// Set during TestPlan parsing as part of overrides and templating.
    #[serde(default)]
    #[schemars(skip)]
    #[doc(hidden)]
    pub(crate) src: Option<SourceDir>,
}

impl Template for RelativeFile {
    fn required_variables(&self) -> Vec<String> {
        self.path.required_variables()
    }

    fn validate_context(
        &self,
        path: &mut Vec<String>,
        allowed_variables: &HashSet<&String>,
        file_source: &SourceDir,
        ctx: &TemplateContext,
    ) -> templating::Result<()> {
        self.path
            .validate_context_nested(path, "path", allowed_variables, file_source, ctx)
    }

    fn try_template(
        &mut self,
        path: &mut Vec<String>,
        file_source: &SourceDir,
        ctx: &TemplateContext,
    ) -> templating::Result<()> {
        try_template_path_and_source(&mut self.path, &mut self.src, path, file_source, ctx)
    }
}

impl AsUtf8FileContent for RelativeFile {
    async fn try_get_file_content(
        &self,
        ctx: &impl ResolutionContext,
    ) -> providers::Result<String> {
        match self.src.as_ref() {
            Some(SourceDir::Local { abs_path }) => {
                let p = abs_path.join(self.path.as_resolved());
                Ok(ctx.read_path_to_string(p)?)
            }

            Some(SourceDir::Github {
                org,
                repo,
                path,
                git_ref,
            }) => {
                let client = ctx.github_client().ok_or(providers::Error::Github(
                    rtf_integrations::github::Error::NoClient,
                ))?;
                let full_path = path.join(self.path.as_resolved()).display().to_string();

                Ok(client
                    .string_file_content(org, repo, &full_path, git_ref.as_ref())
                    .await?)
            }

            None => panic!("attempt to resolve a RelativeFile without a source"),
        }
    }
}

impl Check for RelativeFile {
    fn try_check(
        &self,
        path: &mut Vec<String>,
        ctx: &impl ResolutionContext,
    ) -> checks::Result<()> {
        let file_path = self.path.as_resolved();
        let src = self
            .src
            .as_ref()
            .expect("attempt to check a RelativeFile without a source");

        let p = match check_path(file_path, src, path, ctx)? {
            Some(p) => p,
            None => return Ok(()),
        };

        match ctx.path_kind(&p) {
            PathKind::File => Ok(()),
            PathKind::EmptyDir | PathKind::OccupiedDir => Err(checks::Errors::new(
                checks::ErrorKind::IsADirectory,
                format_error_message(src, file_path),
                path,
            )),
            PathKind::Missing => Err(checks::Errors::new(
                checks::ErrorKind::FileNotFound,
                format_error_message(src, file_path),
                path,
            )),
        }
    }
}

#[inline(always)]
fn format_error_message(src: &SourceDir, path: impl AsRef<Path>) -> String {
    format!("provided path was {}", src.to_uri_for(path))
}

fn check_path(
    str_path: &str,
    src: &SourceDir,
    err_path: &[String],
    ctx: &impl ResolutionContext,
) -> checks::Result<Option<PathBuf>> {
    let res = match src {
        SourceDir::Local { abs_path } => ctx.canonicalize_path(abs_path.join(str_path)).map(Some),

        SourceDir::Github { .. } => {
            return if ctx.github_client().is_none() {
                Err(checks::Errors::new(
                    checks::ErrorKind::MissingGithubApiKey,
                    "expected os env key GITHUB_TOKEN",
                    err_path,
                ))
            } else {
                Ok(None)
            };
        }
    };

    res.map_err(|e| {
        let kind = if e.kind() == io::ErrorKind::NotFound {
            checks::ErrorKind::FileNotFound
        } else {
            checks::ErrorKind::InvalidRelativePath
        };

        checks::Errors::new(kind, format_error_message(src, str_path), err_path)
    })
}

/// Shared logic for templating RelativeFile and RelativeDir
fn try_template_path_and_source(
    path_field: &mut Field<String>,
    src: &mut Option<SourceDir>,
    path: &mut Vec<String>,
    file_source: &SourceDir,
    ctx: &TemplateContext,
) -> templating::Result<()> {
    use templating::{ErrorKind, Errors, ValidField};

    path.push("path".into());

    match path_field {
        // If we're pending then we template and store the source of the variable we used
        Field::Pending(variable) => match ctx.get_with_source(variable) {
            Some((source, raw)) => match String::try_from_scalar(raw.clone()) {
                Ok(path) => {
                    *path_field = Field::Resolved(path);
                    *src = Some(source.clone());
                }
                Err(reason) => {
                    return Err(Errors::new(ErrorKind::InvalidData, reason, path));
                }
            },
            None => {
                return Err(Errors::new(
                    ErrorKind::UnknownVariable,
                    variable.clone(),
                    path,
                ));
            }
        },

        // If we aren't already pinned to a source, we store the source of the file we are in
        Field::Resolved(_) if src.is_none() => *src = Some(file_source.clone()),

        // Otherwise we are resolved and already have a source set
        Field::Resolved(_) => (),
    }

    assert!(
        src.is_some(),
        "should always have a source once we've templated"
    );

    Ok(())
}

/// # Relative dir
///
/// A relative path from the containing config file to a target directory and a list of files
/// that should be made available as part of the test run. This provider works both with local
/// directories and directories within GitHub if the containing config file was pulled from a
/// repository.
///
/// The specified environment variable for this provider will point to the location of the
/// directory itself. Relative paths under the specified directory will be maintained and in
/// order to provided deterministic locations for each included file.
///
/// Only the specified files will be included and there is no way to wildcard multiple files.
///
/// ```yaml
/// - name: "my-data"
///   env_var: MY_DATA
///   kind: relative_dir
///   path: "../../resources/test-data"
///   files:
///     - "my-file.txt"
///     - "nested/my-nested-file.json"
/// ```
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct RelativeDir {
    /// The relative path from the containing config file to the target directory.
    pub(crate) path: Field<String>,

    /// The file paths under this directory that should be included.
    pub(crate) files: Vec<String>,

    /// Set during TestPlan parsing as part of overrides and templating.
    #[serde(default)]
    #[schemars(skip)]
    #[doc(hidden)]
    pub(crate) src: Option<SourceDir>,
}

impl RelativeDir {
    pub fn as_relative_files(&self) -> Vec<RelativeFile> {
        let mut src = self
            .src
            .clone()
            .expect("attempt to resolve a RelativeDir without a source");
        match &mut src {
            SourceDir::Local { abs_path } => *abs_path = abs_path.join(self.path.as_resolved()),
            SourceDir::Github { path, .. } => *path = path.join(self.path.as_resolved()),
        }

        // Dedup to ensure that we don't pull in files multiple times
        let mut files = self.files.clone();
        files.sort_unstable();
        files.dedup();

        files
            .into_iter()
            .map(|fname| RelativeFile {
                path: Field::Resolved(fname),
                src: Some(src.clone()),
            })
            .collect()
    }
}

impl Template for RelativeDir {
    fn required_variables(&self) -> Vec<String> {
        self.path.required_variables()
    }

    fn validate_context(
        &self,
        path: &mut Vec<String>,
        allowed_variables: &HashSet<&String>,
        file_source: &SourceDir,
        ctx: &TemplateContext,
    ) -> templating::Result<()> {
        self.path
            .validate_context_nested(path, "path", allowed_variables, file_source, ctx)
    }

    fn try_template(
        &mut self,
        path: &mut Vec<String>,
        file_source: &SourceDir,
        ctx: &TemplateContext,
    ) -> templating::Result<()> {
        try_template_path_and_source(&mut self.path, &mut self.src, path, file_source, ctx)
    }
}

#[allow(async_fn_in_trait)]
impl ResolveFileContent for RelativeDir {
    async fn try_get_all_file_contents(
        &self,
        target: impl AsRef<Path>,
        ctx: &impl ResolutionContext,
    ) -> providers::Result<Vec<DirFile>> {
        let mut contents = Vec::with_capacity(self.files.len());

        for rel_file in self.as_relative_files() {
            let path = target.as_ref().join(rel_file.path.as_resolved());
            contents.push(DirFile {
                path,
                content: rel_file.try_get_file_content(ctx).await?,
            });
        }

        Ok(contents)
    }
}

impl Check for RelativeDir {
    fn try_check(
        &self,
        path: &mut Vec<String>,
        ctx: &impl ResolutionContext,
    ) -> checks::Result<()> {
        let dir_path = self.path.as_resolved();
        let src = self
            .src
            .as_ref()
            .expect("attempt to check a RelativeDir without a source");

        // Early return on errors here because without the top level path we can't check anything
        // else.
        let p = match check_path(dir_path, src, path, ctx)? {
            Some(p) => p,
            None => return Ok(()),
        };

        match ctx.path_kind(&p) {
            // Allow missing files to be handled below
            PathKind::EmptyDir | PathKind::OccupiedDir => (),

            PathKind::File => {
                return Err(checks::Errors::new(
                    checks::ErrorKind::NotADirectory,
                    format_error_message(src, dir_path),
                    path,
                ));
            }

            PathKind::Missing => {
                return Err(checks::Errors::new(
                    checks::ErrorKind::FileNotFound,
                    format_error_message(src, dir_path),
                    path,
                ));
            }
        };

        // Gather all errors from missing file paths within the directory if we got this far
        let mut errs = checks::ErrorBuilder::new();

        // We check for an empty files array as a hard error as in this instance the file provider
        // does nothing at all, but we don't check for duplicate file paths. When resolving, we
        // ensure that files are only pulled in once so duplicates here don't cause us any problems.
        if self.files.is_empty() {
            errs.push(checks::ErrorKind::EmptyArray, "files", path);
        }

        for (i, fname) in self.files.iter().enumerate() {
            let mut fpath = path.clone();
            fpath.push(i.to_string());

            match check_relative_path_specifiers(Path::new(fname), path) {
                Err(e) => errs.append(Err(e)),
                Ok(()) => match ctx.path_kind(p.join(fname)) {
                    PathKind::File => (),
                    PathKind::EmptyDir | PathKind::OccupiedDir => errs.push(
                        checks::ErrorKind::IsADirectory,
                        format_error_message(src, fname),
                        path,
                    ),
                    PathKind::Missing => errs.push(
                        checks::ErrorKind::FileNotFound,
                        format_error_message(src, fname),
                        path,
                    ),
                },
            }
        }

        errs.into_result(())
    }
}

/// # Required file
///
/// The only purpose of this file provider is to throw an error if it still exists
/// when the file providers are being checked. All definitions of a required file
/// are expected to be replaced by user defined file providers.
///
/// ```yaml
/// - name: "router-config.yaml"
///   env_var: ROUTER_CONFIG
///   kind: required
///   message: "you must specify a router config file to use"
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema, Template)]
pub struct RequiredFile {
    /// The error message to display to the user if this provider is not overwritten.
    #[template(skip)]
    pub(crate) message: String,
}

impl AsUtf8FileContent for RequiredFile {
    async fn try_get_file_content(
        &self,
        _ctx: &impl ResolutionContext,
    ) -> providers::Result<String> {
        panic!(
            "Should not be able to get here. Required file should result in an error when checked."
        )
    }
}

impl Check for RequiredFile {
    fn try_check(
        &self,
        path: &mut Vec<String>,
        _ctx: &impl ResolutionContext,
    ) -> checks::Result<()> {
        Err(checks::Errors::new(
            checks::ErrorKind::RequiredFileMissing,
            &self.message,
            path,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        context::Context,
        mock_context::MockContext,
        providers::test_helpers::{assert_file_content, create_temp_dir_with_file},
        templating::{ErrorKind, Scalar},
    };
    use assert_fs::{
        TempDir,
        assert::PathAssert,
        fixture::{ChildPath, PathChild},
        prelude::FileWriteStr,
    };
    use indoc::indoc;
    use predicates::path;
    use simple_test_case::test_case;
    use std::{path::PathBuf, str::FromStr};

    macro_rules! template_context {
        ($slice:expr) => {{
            let mut m = ::std::collections::HashMap::new();
            for k in $slice {
                m.insert(k.to_string(), Scalar::from(k.to_string()));
            }

            TemplateContext::new_stubbed(m)
        }};
    }

    /// Create a RelativeFile for testing - returns the actual file in a tmp dir
    fn relative_file(path: &str, source: SourceDir) -> RelativeFile {
        RelativeFile {
            path: Field::Resolved(path.to_string()),
            src: Some(source),
        }
    }

    /// Assert check errors
    pub(crate) fn assert_check_errors(
        c: impl Check,
        ctx: &Context,
        expected_err_kinds: &[checks::ErrorKind],
    ) {
        let res = c.try_check(&mut Vec::new(), ctx);
        assert!(res.is_err(), "expected check to fail, got {res:?}");

        let err = res.unwrap_err();
        let err_kinds: Vec<checks::ErrorKind> = err.iter().map(|e| e.kind).collect();
        assert_eq!(
            err_kinds, expected_err_kinds,
            "check the error kind is correct"
        );
    }

    /// Assert resolve and write success
    pub(crate) async fn assert_resolve_and_write_success(
        provider: FileProvider,
        target: &ChildPath,
        ctx: &mut impl ResolutionContext,
        expected_content: &str,
    ) {
        let res = provider.resolve_and_write(target, ctx).await;
        assert!(
            res.is_ok(),
            "expected file to resolve and write, got {res:?}"
        );

        assert_file_content(target, expected_content);
    }

    /// Assert resolve and write error
    pub(crate) async fn assert_resolve_and_write_error(
        provider: FileProvider,
        target: &ChildPath,
        ctx: &mut impl ResolutionContext,
        expected_err_str: &str,
    ) {
        let res = provider.resolve_and_write(target, ctx).await;
        assert!(
            res.is_err(),
            "expected file to fail to resolve and write, got {res:?}"
        );

        target.assert(path::missing());
        let err = res.unwrap_err();
        assert_eq!(err.to_string(), expected_err_str)
    }

    // Yaml snippets for all file providers
    const BUILD_ROUTER_FROM_SOURCE: &str = indoc!(
        r#"
        kind: build_router_from_source
        git_ref: "{{ git_ref }}"
        rust_version: "{{ rust_version }}"
        profile: "{{ profile }}"
        features: "{{ features }}"
    "#
    );
    const CONDITIONAL: &str = indoc!(
        r#"
        kind: conditional
        cases:
          - where: { var: test_type, eq: load }
            kind: relative_path
            path: "{{ case_1 }}"
          - where: { var: test_type, ne: ramp }
            kind: relative_path
            path: "{{ case_2 }}"
    "#
    );
    const CUSTOM_PROVIDER_YAML: &str = indoc!(
        r#"
        kind: custom_provider
        type: "my-custom-provider"
        key1: "{{ value1 }}"
        key2: "value2"
        key3: "value3"
        "#
    );
    const GITHUB_FILE: &str = indoc!(
        r#"
        kind: github_file
        org: "{{ org }}"
        repo: "{{ repo }}"
        path: "{{ path }}"
        git_ref: "{{ git_ref }}"
    "#
    );
    const GRAPHOS_CANNED_OPS: &str = indoc!(
        r#"
        kind: graphos_canned_ops
        graph_ref: "{{ graph_ref }}"
        top_n: "{{ top_n }}"
        skip_mutations: "{{ skip_mutations }}"
        time_range: "{{ time_range }}"
    "#
    );
    const GRAPHOS_CANNED_OPS_BY_ID: &str = indoc!(
        r#"
        kind: graphos_canned_ops_by_id
        graph_ref: "{{ graph_ref }}"
        operation_ids:
          - "{{ op_1 }}"
          - "{{ op_2 }}"
    "#
    );
    const GRAPHOS_SUBGRAPH_ROUTER_URL_OVERRIDES: &str = indoc!(
        r#"
        kind: graphos_subgraph_router_url_overrides
        graph_ref: "{{ graph_ref }}"
    "#
    );
    const GRAPHOS_SUBGRAPHS: &str = indoc!(
        r#"
        kind: graphos_subgraphs
        graph_ref: "{{ graph_ref }}"
    "#
    );
    const GRAPHOS_SUBGRAPH_NAMES: &str = indoc!(
        r#"
        kind: graphos_subgraph_names
        graph_ref: "{{ graph_ref }}"
    "#
    );
    const GRAPHOS_SUPERGRAPH: &str = indoc!(
        r#"
        kind: graphos_supergraph
        graph_ref: "{{ graph_ref }}"
    "#
    );
    const FROM_COMMAND: &str = indoc!(
        r#"
        kind: from_command
        command:
          name: test.sh
          kind: relative_path
          path: "{{ test_script }}"
          args:
            - "{{ test_arg }}"
        env_vars:
          TEST_VAR: "{{ test_var }}"
        file_providers:
          - name: test-file.txt
            env_var: TEST_FILE
            kind: relative_path
            path: "{{ test_path }}"
    "#
    );
    const INLINE: &str = indoc!(
        r#"
        kind: inline
        content: |
            some content
    "#
    );
    const INLINE_DIR: &str = indoc!(
        r#"
        kind: inline_dir
        files:
          - path: file1
            content: |
              some content for file1
          - path: nested/file2
            content: |
              some content for file2
    "#
    );
    const OFFLINE_GRAPHOS_LICENSE: &str = indoc!(
        r#"
        kind: offline_graphos_license
        graph_id: "{{ graph_id }}"
    "#
    );
    const RELATIVE_DIR: &str = indoc!(
        r#"
        kind: relative_dir
        path: "{{ path }}"
        files:
          - "foo"
          - "bar"
    "#
    );
    const RELATIVE_PATH: &str = indoc!(
        r#"
        kind: relative_path
        path: "{{ path }}"
    "#
    );
    const REQUIRED_FILE: &str = indoc!(
        r#"
        kind: required
        message: this file is required
    "#
    );
    const ROUTER_DOWNLOAD_SCRIPT: &str = indoc!(
        r#"
        kind: router_download_script
        version: "{{ version }}"
    "#
    );
    const MERGE_YAML: &str = indoc!(
        r#"
        kind: merge_yaml
        base:
          kind: inline
          content: |
            key: value
        overrides:
          kind: inline
          content: |
            new_key: new_value
    "#
    );
    const MERGE_YAML_ARRAY: &str = indoc!(
        r#"
        kind: merge_yaml
        base:
          kind: inline
          content: |
            key: value
        overrides:
          - kind: inline
            content: |
              new_key: new_value
          - kind: inline
            content: |
              another_new_key: another_new_value
    "#
    );
    const TEMPLATED: &str = indoc!(
        r#"
        kind: templated
        content: |
          { "endpoint": "${router_url}", "key": "${api_key}" }
    "#
    );

    #[test_case(BUILD_ROUTER_FROM_SOURCE, &["git_ref", "rust_version", "profile", "features"]; "build_router_from_source")]
    #[test_case(CONDITIONAL, &["case_1", "case_2", "test_type"]; "conditional")]
    #[test_case(CUSTOM_PROVIDER_YAML, &["value1"]; "custom_provider")]
    #[test_case(GITHUB_FILE, &["org", "repo", "path", "git_ref"]; "github_file")]
    #[test_case(GRAPHOS_CANNED_OPS, &["graph_ref", "top_n", "skip_mutations", "time_range"]; "graphos_canned_ops")]
    #[test_case(GRAPHOS_CANNED_OPS_BY_ID, &["graph_ref", "op_1", "op_2"]; "graphos_canned_ops_by_id")]
    #[test_case(GRAPHOS_SUBGRAPH_ROUTER_URL_OVERRIDES, &["graph_ref"]; "graphos_subgraph_router_url_overrides")]
    #[test_case(GRAPHOS_SUBGRAPHS, &["graph_ref"]; "graphos_subgraphs")]
    #[test_case(GRAPHOS_SUBGRAPH_NAMES, &["graph_ref"]; "graphos_subgraph_names")]
    #[test_case(GRAPHOS_SUPERGRAPH, &["graph_ref"]; "graphos_supergraph")]
    #[test_case(FROM_COMMAND, &["test_script", "test_arg", "test_var", "test_path"]; "from_command")]
    #[test_case(INLINE, &[]; "inline")]
    #[test_case(INLINE_DIR, &[]; "inline_dir")]
    #[test_case(OFFLINE_GRAPHOS_LICENSE, &["graph_id"]; "offline_graphos_license")]
    #[test_case(RELATIVE_DIR, &["path"]; "relative_dir")]
    #[test_case(RELATIVE_PATH, &["path"]; "relative_path")]
    #[test_case(REQUIRED_FILE, &[]; "required")]
    #[test_case(ROUTER_DOWNLOAD_SCRIPT, &["version"]; "router_download_script")]
    #[test_case(MERGE_YAML, &[]; "merge_yaml")]
    #[test_case(MERGE_YAML_ARRAY, &[]; "merge_yaml_array")]
    #[test_case(TEMPLATED, &["router_url", "api_key"]; "templated")]
    #[test]
    fn all_fields_templated(content: &str, expected_variables: &[&str]) {
        let config: FileProvider = serde_yaml::from_str(content).unwrap();

        let res = config.required_variables();
        assert_eq!(res, expected_variables, "expected variables to match")
    }

    #[test_case(Field::Pending("foo".to_string()), &["foo"]; "field is required")]
    #[test_case(Field::Resolved("foo".to_string()), &[]; "no fields required")]
    #[test]
    fn named_file_provider_required_variables(f: Field<String>, expected: &[&str]) {
        let nfp = NamedFileProvider {
            name: "inline.txt".to_string(),
            env_var: "INLINE".to_string(),
            provider: FileProvider::RelativePath(RelativeFile { path: f, src: None }),
        };

        let res = nfp.required_variables();
        assert_eq!(
            res, expected,
            "tests that required_variables has expected value"
        )
    }

    #[test]
    fn named_file_provider_try_template_succeeds() {
        let mut nfp = NamedFileProvider {
            name: "inline.txt".to_string(),
            env_var: "INLINE".to_string(),
            provider: FileProvider::RelativePath(RelativeFile {
                path: Field::Pending("path".to_string()),
                src: None,
            }),
        };
        let ctx = template_context!(&["path"]);

        let res = nfp.try_template(&mut Vec::new(), &SourceDir::local("/"), &ctx);
        assert!(
            res.is_ok(),
            "expected to template successfully, got {res:?}"
        )
    }

    #[test]
    fn named_file_provider_try_template_unknown_variable_error() {
        let mut nfp = NamedFileProvider {
            name: "relative.txt".to_string(),
            env_var: "RELATIVE".to_string(),
            provider: FileProvider::RelativePath(RelativeFile {
                path: Field::Pending("path".to_string()),
                src: None,
            }),
        };
        let ctx = template_context!(&["unused"]);

        let res = nfp.try_template(&mut vec!["path".to_string()], &SourceDir::local("/"), &ctx);
        assert!(res.is_err(), "expected templating to error, got {res:?}");

        let errors = res.unwrap_err();
        let error = errors.unwrap_single();
        let error_kind = error.kind;
        let error_path = error.path;
        assert_eq!(
            error_kind,
            ErrorKind::UnknownVariable,
            "expected ErrorKind to match"
        );
        assert_eq!(error_path, "path.RELATIVE.path", "expected path to match")
    }

    #[test]
    fn named_file_provider_check_error_path_correct() {
        let nfp = NamedFileProvider {
            name: "relative.txt".to_string(),
            env_var: "RELATIVE".to_string(),
            provider: FileProvider::RelativePath(RelativeFile {
                path: Field::Resolved("does/not/exist/relative.txt".to_string()),
                src: Some(SourceDir::local("/foo")),
            }),
        };

        let res = nfp.try_check(&mut vec!["path".to_string()], &Context::new());
        assert!(res.is_err(), "expected to check to error, got {res:?}");
        let err = res.unwrap_err().unwrap_single();
        assert_eq!(err.path, "path.RELATIVE")
    }

    #[test]
    fn named_file_provider_check_error_name_path_invalid() {
        let nfp = NamedFileProvider {
            name: "../inline.txt".to_string(),
            env_var: "INLINE".to_string(),
            provider: FileProvider::Inline(InlineFile {
                content: "content".to_string(),
            }),
        };

        let res = nfp.try_check(&mut vec!["path".to_string()], &Context::new());
        assert!(res.is_err(), "expected to check to error, got {res:?}");
        let err = res.unwrap_err().unwrap_single();
        assert_eq!(err.path, "path.INLINE");
        assert_eq!(err.kind, checks::ErrorKind::InvalidPathSpecifiers)
    }

    #[test]
    fn inline_file_check_success() {
        let inline = InlineFile {
            content: "some content".to_string(),
        };

        let ctx = Context::new();

        let res = inline.try_check(&mut Vec::new(), &ctx);
        assert!(res.is_ok(), "expected check to succeed, got {res:?}")
    }

    #[test]
    fn inline_dir_check_success() {
        let inline = InlineDir {
            files: vec![DirFile {
                path: PathBuf::from_str("path/to/file.txt").unwrap(),
                content: "file content".to_string(),
            }],
        };

        let ctx = Context::new();

        let res = inline.try_check(&mut Vec::new(), &ctx);
        assert!(res.is_ok(), "expected check to succeed, got {res:?}")
    }

    #[test]
    fn inline_dir_check_invalid_relative_path_errors() {
        let inline = InlineDir {
            files: vec![
                DirFile {
                    path: PathBuf::from_str("./file.txt").unwrap(),
                    content: "file content".to_string(),
                },
                DirFile {
                    path: PathBuf::from_str("../file.txt").unwrap(),
                    content: "file content".to_string(),
                },
                DirFile {
                    path: PathBuf::from_str("/file.txt").unwrap(),
                    content: "file content".to_string(),
                },
            ],
        };
        let ctx = Context::new();

        assert_check_errors(
            inline,
            &ctx,
            &[
                checks::ErrorKind::InvalidPathSpecifiers,
                checks::ErrorKind::InvalidPathSpecifiers,
                checks::ErrorKind::InvalidPathSpecifiers,
            ],
        );
    }

    #[test]
    fn relative_file_check_local_success() {
        let file_name = "file.txt";
        let (temp, _) = create_temp_dir_with_file(file_name, "some content");

        let ctx = Context::new();
        let relative_file = relative_file(file_name, SourceDir::local(temp.path()));

        let res = relative_file.try_check(&mut Vec::new(), &ctx);
        assert!(res.is_ok(), "expected check to succeed, got {res:?}")
    }

    #[test]
    fn relative_file_check_github_success() {
        // This test works because all that's needed for success in the GitHub case is
        // a GitHub token to be defined in the context
        let relative_file = relative_file(
            "file.txt",
            SourceDir::github("org", "repo", "path", Some("ref")),
        );

        let mut ctx = Context::new();
        ctx.with_github_config("dummy_token");

        let res = relative_file.try_check(&mut Vec::new(), &ctx);
        assert!(res.is_ok(), "expected check to succeed, got {res:?}")
    }

    #[test]
    fn relative_file_check_does_not_exist() {
        let relative_file = relative_file("does-not-exist.txt", SourceDir::local("/foo"));
        let ctx = Context::new();

        assert_check_errors(relative_file, &ctx, &[checks::ErrorKind::FileNotFound]);
    }

    #[test]
    fn relative_file_check_is_directory() {
        let (temp, _file) = create_temp_dir_with_file("dir/file.txt", "some content");

        let ctx = Context::new();
        let relative_file = relative_file(
            "dir",
            SourceDir::local(ctx.canonicalize_path(temp.path()).unwrap()),
        );

        assert_check_errors(relative_file, &ctx, &[checks::ErrorKind::IsADirectory]);
    }

    #[test]
    fn relative_file_check_missing_github_api_token() {
        let relative_file = relative_file(
            "file.txt",
            SourceDir::github("org", "repo", "path", Some("ref")),
        );

        // There is no github client added to this context so this fails
        let ctx = Context::new();

        assert_check_errors(
            relative_file,
            &ctx,
            &[checks::ErrorKind::MissingGithubApiKey],
        );
    }

    fn relative_dir(path: &str, files: &[&str], source: SourceDir) -> RelativeDir {
        RelativeDir {
            path: Field::Resolved(path.to_string()),
            files: files.iter().map(|s| s.to_string()).collect(),
            src: Some(source),
        }
    }

    #[test]
    fn relative_dir_check_local_success() {
        let file_name = "my-dir/file.txt";
        let (temp, _) = create_temp_dir_with_file(file_name, "some content");

        let ctx = Context::new();
        let relative_dir = relative_dir("my-dir", &["file.txt"], SourceDir::local(temp.path()));
        let res = relative_dir.try_check(&mut Vec::new(), &ctx);

        assert!(res.is_ok(), "expected check to succeed, got {res:?}")
    }

    #[test]
    fn relative_dir_check_github_success() {
        // This test works because all that's needed for success in the GitHub case is
        // a GitHub token to be defined in the context
        let relative_dir = relative_dir(
            "my-dir",
            &["file.txt"],
            SourceDir::github("org", "repo", "path", Some("ref")),
        );

        let mut ctx = Context::new();
        ctx.with_github_config("dummy_token");

        let res = relative_dir.try_check(&mut Vec::new(), &ctx);
        assert!(res.is_ok(), "expected check to succeed, got {res:?}")
    }

    #[test]
    fn relative_dir_check_does_not_exist() {
        let relative_dir = relative_dir("nope", &["not-here.txt"], SourceDir::local("/foo"));
        let ctx = Context::new();

        assert_check_errors(relative_dir, &ctx, &[checks::ErrorKind::FileNotFound]);
    }

    #[test]
    fn relative_dir_check_is_file() {
        let (temp, _file) = create_temp_dir_with_file("dir/file.txt", "some content");

        let ctx = Context::new();
        let relative_dir = relative_dir(
            "dir/file.txt",
            &["oh-no"],
            SourceDir::local(ctx.canonicalize_path(temp.path()).unwrap()),
        );

        assert_check_errors(relative_dir, &ctx, &[checks::ErrorKind::NotADirectory]);
    }

    #[test]
    fn relative_dir_check_empty_files_array() {
        let (temp, _) = create_temp_dir_with_file("my-dir/foo.txt", "some content");

        let ctx = Context::new();
        let relative_dir = relative_dir("my-dir", &[], SourceDir::local(temp.path()));

        assert_check_errors(relative_dir, &ctx, &[checks::ErrorKind::EmptyArray]);
    }

    #[test]
    fn relative_dir_check_missing_github_api_token() {
        let relative_dir = relative_dir(
            "my-dir",
            &["file.txt"],
            SourceDir::github("org", "repo", "path", Some("ref")),
        );

        // There is no github client added to this context so this fails
        let ctx = Context::new();

        assert_check_errors(
            relative_dir,
            &ctx,
            &[checks::ErrorKind::MissingGithubApiKey],
        );
    }

    #[test]
    fn required_file_check_is_missing() {
        let required_file = RequiredFile {
            message: "need to override".to_string(),
        };

        let ctx = Context::new();

        // Not reusing assert_check_error so the message can also be checked
        let res = required_file.try_check(&mut Vec::new(), &ctx);
        assert!(res.is_err(), "expected check to fail, got {res:?}");

        let err = res.unwrap_err().unwrap_single();
        assert_eq!(
            err.kind,
            checks::ErrorKind::RequiredFileMissing,
            "check the error kind is correct"
        );
        assert_eq!(
            required_file.message, err.message,
            "check the error message matches the provider message"
        );
    }

    #[tokio::test]
    async fn inline_file_resolve_and_write_success() {
        let temp = TempDir::new().unwrap();
        let target = temp.child("inline.txt");

        let mut ctx = Context::new();
        let expected_content = "some content";
        let inline = FileProvider::Inline(InlineFile {
            content: expected_content.to_string(),
        });

        assert_resolve_and_write_success(inline, &target, &mut ctx, expected_content).await;
    }

    #[tokio::test]
    async fn inline_dir_resolve_and_write_success() {
        let temp = TempDir::new().unwrap();
        let target = temp.child("directory");

        let file1_path = "file1.txt";
        let file1_content = "file1 content";
        let file2_path = "nested/file2.txt";
        let file2_content = "file2 content";
        let file3_path = "some/very/deeply/nested/file3.txt";
        let file3_content = "file3 content";

        let mut ctx = Context::new();
        let inline_dir = FileProvider::InlineDir(InlineDir {
            files: vec![
                DirFile {
                    path: PathBuf::from_str(file1_path).unwrap(),
                    content: file1_content.to_string(),
                },
                DirFile {
                    path: PathBuf::from_str(file2_path).unwrap(),
                    content: file2_content.to_string(),
                },
                DirFile {
                    path: PathBuf::from_str(file3_path).unwrap(),
                    content: file3_content.to_string(),
                },
            ],
        });

        let res = inline_dir.resolve_and_write(&target, &mut ctx).await;
        assert!(
            res.is_ok(),
            "expected file to resolve and write, got {res:?}"
        );

        target.assert(path::exists());
        assert_file_content(&target.child(file1_path), file1_content);
        assert_file_content(&target.child(file2_path), file2_content);
        assert_file_content(&target.child(file3_path), file3_content);
    }

    #[tokio::test]
    async fn relative_file_resolve_and_write_local_success() {
        let expected_content = "example file content";
        let (temp, _file_to_read) = create_temp_dir_with_file("file.txt", expected_content);
        let test_plan_file = temp.child("test-plan.yaml");
        test_plan_file
            .write_str("empty test plan")
            .expect("unable to write test plan");

        let target = temp.child("output/relative.txt");

        let mut ctx = Context::new();
        // Relative File paths are resolved relative to the test plan file's location.
        // We have created an empty test plan file so we can canonicalize its path (it must exist for this to work)
        // This allows us to read the relative file from the correct path
        let src = SourceDir::local(ctx.canonicalize_path(temp.path()).unwrap());
        let relative = FileProvider::RelativePath(relative_file("file.txt", src.clone()));

        assert_resolve_and_write_success(relative, &target, &mut ctx, expected_content).await
    }

    #[tokio::test]
    async fn relative_file_resolve_and_write_github_success() {
        let temp = TempDir::new().unwrap();
        let target = temp.child("output/relative.txt");

        let content = "some content";

        let mut ctx = MockContext::with_github_client(&[("org/repo/my-tests/file.txt", content)]);
        let src = SourceDir::Github {
            org: "org".to_string(),
            repo: "repo".to_string(),
            path: "my-tests".into(),
            git_ref: None,
        };

        // The file does not exist but this does not matter since we mock a GitHub response
        let relative = FileProvider::RelativePath(relative_file("file.txt", src));

        assert_resolve_and_write_success(relative, &target, &mut ctx, content).await
    }

    #[tokio::test]
    async fn relative_file_resolve_and_write_local_does_not_exist() {
        let temp = TempDir::new().unwrap();
        let target = temp.child("output/relative.txt");

        let mut ctx = Context::new();
        let src = SourceDir::Local {
            abs_path: ctx.canonicalize_path(&temp).unwrap(),
        };

        let expected_err = "No such file or directory (os error 2)";

        // The file.txt file has not been created in temp
        let relative = FileProvider::RelativePath(relative_file("file.txt", src));

        assert_resolve_and_write_error(relative, &target, &mut ctx, expected_err).await
    }

    #[tokio::test]
    async fn relative_file_resolve_and_write_local_is_not_text() {
        let temp = TempDir::new().unwrap();
        let target = temp.child("file.txt");

        let crate_root_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));

        let mut ctx = Context::new();
        let src = SourceDir::Local {
            abs_path: ctx
                .canonicalize_path(crate_root_path.join("resources"))
                .unwrap(),
        };

        let expected_err = "stream did not contain valid UTF-8";

        // The frog-no.gif cannot be read since the file to read is not utf-8
        let relative = FileProvider::RelativePath(relative_file("frog-no.gif", src));

        assert_resolve_and_write_error(relative, &target, &mut ctx, expected_err).await
    }

    #[tokio::test]
    async fn relative_file_resolve_and_write_github_no_github_client() {
        let temp = TempDir::new().unwrap();
        let target = temp.child("output/relative.txt");

        let mut ctx = Context::new();
        let src = SourceDir::Github {
            org: "org".to_string(),
            repo: "repo".to_string(),
            path: "path".into(),
            git_ref: None,
        };

        let expected_err = "no GitHub client available";

        // The file does not exist but this does not matter since we mock a GitHub response
        let relative = FileProvider::RelativePath(relative_file("file.txt", src));

        assert_resolve_and_write_error(relative, &target, &mut ctx, expected_err).await
    }

    fn create_temp_dir_with_files(files: &[(&str, &str)]) -> TempDir {
        let temp = TempDir::new().unwrap();

        for (file_path, file_content) in files.iter() {
            let file = temp.child(file_path);
            file.write_str(file_content).unwrap();
        }

        temp
    }

    #[tokio::test]
    async fn relative_dir_resolve_and_write_local_success() {
        let files = &[
            ("relative/one.txt", "foo"),
            ("relative/nested/two.txt", "bar"),
        ];

        let temp = create_temp_dir_with_files(files);
        let output = temp.child("output");

        let mut ctx = Context::new();
        let src = SourceDir::local(ctx.canonicalize_path(temp.path()).unwrap());
        let relative = FileProvider::RelativeDir(relative_dir(
            "relative",
            &["one.txt", "nested/two.txt"],
            src,
        ));

        let res = relative.resolve_and_write(&output, &mut ctx).await;
        assert!(res.is_ok(), "expected Ok, got {res:?}");

        assert_file_content(&output.child("one.txt"), "foo");
        assert_file_content(&output.child("nested/two.txt"), "bar");
    }

    #[tokio::test]
    async fn relative_dir_resolve_and_write_github_success() {
        let temp = TempDir::new().unwrap();
        let output = temp.child("output");

        let mut ctx = MockContext::with_github_client(&[
            ("org/repo/data/one.txt", "foo"),
            ("org/repo/data/two.txt", "bar"),
        ]);
        let src = SourceDir::Github {
            org: "org".to_string(),
            repo: "repo".to_string(),
            path: "".into(),
            git_ref: None,
        };

        let relative =
            FileProvider::RelativeDir(relative_dir("data", &["one.txt", "two.txt"], src));

        let res = relative.resolve_and_write(&output, &mut ctx).await;
        assert!(res.is_ok(), "expected Ok, got {res:?}");

        assert_file_content(&output.child("one.txt"), "foo");
        assert_file_content(&output.child("two.txt"), "bar");
    }

    #[tokio::test]
    async fn relative_dir_resolve_and_write_local_does_not_exist() {
        let temp = create_temp_dir_with_files(&[]);
        let output = temp.child("output");

        let mut ctx = Context::new();
        let src = SourceDir::local(ctx.canonicalize_path(temp.path()).unwrap());
        let relative =
            FileProvider::RelativeDir(relative_dir("relative", &["one.txt", "two.txt"], src));

        let expected_err = "No such file or directory (os error 2)";
        assert_resolve_and_write_error(relative, &output, &mut ctx, expected_err).await
    }

    #[tokio::test]
    async fn relative_dir_resolve_and_write_local_is_not_text() {
        let temp = TempDir::new().unwrap();
        let output = temp.child("output");

        let crate_root_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let mut ctx = Context::new();
        let src = SourceDir::Local {
            abs_path: ctx.canonicalize_path(crate_root_path).unwrap(),
        };

        // The frog-no.gif cannot be read since the file to read is not utf-8
        let relative = FileProvider::RelativeDir(relative_dir("resources", &["frog-no.gif"], src));

        let expected_err = "stream did not contain valid UTF-8";
        assert_resolve_and_write_error(relative, &output, &mut ctx, expected_err).await
    }

    #[tokio::test]
    async fn relative_dir_resolve_and_write_github_no_github_client() {
        let temp = TempDir::new().unwrap();

        let mut ctx = Context::new();
        let src = SourceDir::Github {
            org: "org".to_string(),
            repo: "repo".to_string(),
            path: "".into(),
            git_ref: None,
        };

        let relative = FileProvider::RelativeDir(relative_dir("my-dir", &["one.txt"], src));

        let expected_err = "no GitHub client available";
        assert_resolve_and_write_error(relative, &temp.child("my-dir"), &mut ctx, expected_err)
            .await
    }

    #[tokio::test]
    #[should_panic(
        expected = "Should not be able to get here. Required file should result in an error when checked."
    )]
    async fn required_file_inline_all_files_panics() {
        let ctx = Context::new();
        let mut required = FileProvider::Required(RequiredFile {
            message: "required file must be defined".to_string(),
        });

        let _res = required.inline(&InlineMode::All, &ctx).await;
    }

    #[tokio::test]
    #[should_panic(
        expected = "Should not be able to get here. Required file should result in an error when checked."
    )]
    async fn required_file_resolve_and_write_panics() {
        let mut ctx = Context::new();
        let required = FileProvider::Required(RequiredFile {
            message: "required file must be defined".to_string(),
        });

        let _res = required
            .resolve_and_write(Path::new("required.txt"), &mut ctx)
            .await;
    }

    #[tokio::test]
    async fn relative_path_inline_succeeds() {
        let file_content = "example file content";
        let (temp, _file_to_read) = create_temp_dir_with_file("file.txt", file_content);

        let ctx = Context::new();
        let src = SourceDir::local(ctx.canonicalize_path(temp.path()).unwrap());

        let mut file_provider = FileProvider::RelativePath(relative_file("file.txt", src));
        let result = file_provider.inline(&InlineMode::All, &ctx).await;

        assert!(result.is_ok(), "Expected inline to succeed, got {result:?}");
        assert_eq!(
            file_provider,
            FileProvider::Inline(InlineFile {
                content: file_content.to_string(),
            }),
            "Expected file provider to be inlined"
        );
    }

    #[tokio::test]
    async fn file_provider_inline_all_relative_paths_succeeds_for_relative_path() {
        let ctx = Context::new();
        let file_content = "example file content";
        let (temp, _file_to_read) = create_temp_dir_with_file("file.txt", file_content);
        let src = SourceDir::local(ctx.canonicalize_path(temp.path()).unwrap());

        let mut file_provider = FileProvider::RelativePath(relative_file("file.txt", src.clone()));
        let result = file_provider.inline(&InlineMode::RelativeFiles, &ctx).await;

        assert!(
            result.is_ok(),
            "Expected inline_relative_path_provider to succeed, got {result:?}"
        );
        // Assert that the result is an inline file provider with the same contents as the original relative path provider
        let expected_inline_file_provider = FileProvider::Inline(InlineFile {
            content: file_content.to_string(),
        });
        assert_eq!(
            expected_inline_file_provider, file_provider,
            "Expected inline_relative_path_provider to update the file provider to an inline file provider with the same contents as the original relative path provider"
        );
    }

    #[tokio::test]
    async fn file_provider_inline_all_relative_paths_succeeds_for_inline_file() {
        let ctx = Context::new();
        let file_content = "example file content";
        let mut file_provider = FileProvider::Inline(InlineFile {
            content: file_content.to_string(),
        });
        let expected_file_provider = file_provider.clone();

        let result = file_provider.inline(&InlineMode::RelativeFiles, &ctx).await;
        assert!(
            result.is_ok(),
            "Expected inline_relative_path_provider to succeed, got {result:?}"
        );
        assert_eq!(
            expected_file_provider, file_provider,
            "Expected the inline file provider to remain unchanged"
        );
    }
}
