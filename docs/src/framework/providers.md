# Providers

## File Provider

> **NOTE**: These are placeholder docs while we work on setting up an automated way of generating
> these from the JSON schema for the config file types.

### Variants

#### <a id="definitions/BuildRouterFromSource"></a>BuildRouterFromSource

A file provider used for building the Router from source at a specific git commit or reference.

- <a id="definitions/BuildRouterFromSource/properties/commit_ref"></a>**`commit_ref`**: A git
  reference that can be passed to `git checkout`. This may be a full or partial commit hash, branch
  name, or tag.<br> Defaults to `"main"` if unset.
  - **Any of**
    - <a id="definitions/BuildRouterFromSource/properties/commit_ref/anyOf/0"></a>
      _[Templatable string](#definitions/Templatable%2520string)_.
    - <a id="definitions/BuildRouterFromSource/properties/commit_ref/anyOf/1"></a>_null_
- <a id="definitions/BuildRouterFromSource/properties/rust_version"></a>**`rust_version`**: A Rust
  version string that can be passed to `rustup run {rust_version}`, such as `"1.78.0"`, `"beta"`, or
  `"nightly"`.<br> Defaults to `"stable"` if unset.
  - **Any of**
    - <a id="definitions/BuildRouterFromSource/properties/rust_version/anyOf/0"></a>
      _[Templatable string](#definitions/Templatable%2520string)_.
    - <a id="definitions/BuildRouterFromSource/properties/rust_version/anyOf/1"></a>_null_

#### <a id="definitions/GithubFile"></a>GithubFile

The user specifies a path to a file within a GitHub repository, optionally providing a specific ref
of the repository to pull the file from. If no ref is providing then the provider will pull the
version of the file found on the default branch.

- <a id="definitions/GithubFile/properties/org"></a>**`org`** _(required)_: The GitHub org for the
  repository containing the target file. Refer to
  _[Templatable string](#definitions/Templatable%2520string)_.
- <a id="definitions/GithubFile/properties/repo"></a>**`repo`** _(required)_: The GitHub repository
  containing the target file. Refer to _[Templatable string](#definitions/Templatable%2520string)_.
- <a id="definitions/GithubFile/properties/path"></a>**`path`** _(required)_: The absolute path from
  the root of the repository to the target file. Refer to
  _[Templatable string](#definitions/Templatable%2520string)_.
- <a id="definitions/GithubFile/properties/git_ref"></a>**`git_ref`**: An optional git reference to
  pull the file from. This may be a full or partial commit hash, branch name, or tag.<br> Defaults
  to the mainline branch as specified in GitHub if unset.
  - **Any of**
    - <a id="definitions/GithubFile/properties/git_ref/anyOf/0"></a>
      _[Templatable string](#definitions/Templatable%2520string)_.
    - <a id="definitions/GithubFile/properties/git_ref/anyOf/1"></a>_null_

#### <a id="definitions/GraphosCannedOps"></a>GraphosCannedOps

The user specifies the graph ref and parameters that should be used to generate canned GraphQL
requests based on operations data obtained from the GraphOS API.

- <a id="definitions/GraphosCannedOps/properties/graph_ref"></a>**`graph_ref`** _(required)_: The
  Apollo graph ref to pull operations for. Refer to
  _[Templatable string](#definitions/Templatable%2520string)_.
- <a id="definitions/GraphosCannedOps/properties/top_n"></a>**`top_n`**: The number of operations to
  attempt to fetch.<br> Defaults to 20 if unset. Refer to
  _[Templatable integer](#definitions/Templatable%2520integer)_. Default: `20`.
- <a id="definitions/GraphosCannedOps/properties/skip_mutations"></a>**`skip_mutations`**: Whether
  or not to include mutations in the returned operations.<br> Defaults to false if unset. Refer to
  _[Templatable boolean](#definitions/Templatable%2520boolean)_. Default: `false`.

#### <a id="definitions/GraphosSubgraphs"></a>GraphosSubgraphs

The user specifies the graph ref that should be used to fetch a subgraph SDL files from the GraphOS
API.<br> Note that this file proivider will output a directory of SDL schema files, one for each
subgraph.

- <a id="definitions/GraphosSubgraphs/properties/graph_ref"></a>**`graph_ref`** _(required)_: The
  Apollo graph ref to pull subgraph SDL files for. Refer to
  _[Templatable string](#definitions/Templatable%2520string)_.

#### <a id="definitions/GraphosSupergraph"></a>GraphosSupergraph

The user specifies the ref that should be used to fetch a supergraph SDL file from the GraphOS API.

- <a id="definitions/GraphosSupergraph/properties/graph_ref"></a>**`graph_ref`** _(required)_: The
  Apollo graph ref to pull supergraph SDL for. Refer to
  _[Templatable string](#definitions/Templatable%2520string)_.

#### <a id="definitions/InlineFile"></a>InlineFile

The simplest form of file provider: the user specifies the contents of the file inline within their
config file.

- <a id="definitions/InlineFile/properties/content"></a>**`content`** _(string, required)_: The text
  to write out as the contents of the generated file.

#### <a id="definitions/OfflineGraphosLicense"></a>OfflineGraphosLicense

The user specifies the graph id that should be used to fetch an offline license from the GraphOS
API.

- <a id="definitions/OfflineGraphosLicense/properties/graph_id"></a>**`graph_id`** _(required)_: The
  Apollo graph ref to pull an offline license for. Refer to
  _[Templatable string](#definitions/Templatable%2520string)_.

#### <a id="definitions/RelativeFile"></a>RelativeFile

A relative path from the containing config file to a target file that should be made available as
part of the test run. This provider works both with local files and files within GitHub if the
containing config file was pulled from a repository.

- <a id="definitions/RelativeFile/properties/path"></a>**`path`** _(required)_: The relative path
  from the containing config file to the target file. Refer to
  _[Templatable string](#definitions/Templatable%2520string)_.

#### <a id="definitions/RequiredFile"></a>RequiredFile

The only purpose of this file provider is to throw an error if it still exists when the file
providers are being checked. All definitions of a required file are expected to be replaced by user
defined file providers.

- <a id="definitions/RequiredFile/properties/message"></a>**`message`** _(string, required)_: The
  error message to display to the user if this provider is not overwritten.

#### <a id="definitions/RouterDownloadScript"></a>RouterDownloadScript

Produces a POSIX shell script that can be run in order to download a target version of the Apollo
Router.

- <a id="definitions/RouterDownloadScript/properties/version"></a>**`version`** _(required)_: The
  version of the Apollo Router to download. Refer to
  _[Templatable string](#definitions/Templatable%2520string)_.

#### <a id="definitions/MergeYaml"></a>MergeYaml

Merge the YAML output of two text based file providers into a single YAML file.<br> Matching keys in
the overrides file will replace scalar values, concatenate arrays and merge keys for maps.

- <a id="definitions/MergeYaml/properties/base"></a>**`base`** _(required)_: A base YAML file to
  start with. Refer to _[TextFileProvider](#definitions/TextFileProvider)_.
- <a id="definitions/MergeYaml/properties/overrides"></a>**`overrides`** _(required)_: An second
  YAML file to merge on top of the base file. Refer to
  _[TextFileProvider](#definitions/TextFileProvider)_.

#### <a id="definitions/TextFileProvider"></a>TextFileProvider

A subset of file providers that can produce arbitrary utf-8 text as their output.

- **One of**
  - <a id="definitions/TextFileProvider/oneOf/0"></a>_object_
    _[GithubFile](#definitions/GithubFile)_.
    - <a id="definitions/TextFileProvider/oneOf/0/properties/kind"></a>**`kind`** _(string,
      required)_: Must be: `"github_file"`.
  - <a id="definitions/TextFileProvider/oneOf/1"></a>_object_
    _[InlineFile](#definitions/InlineFile)_.
    - <a id="definitions/TextFileProvider/oneOf/1/properties/kind"></a>**`kind`** _(string,
      required)_: Must be: `"inline"`.
  - <a id="definitions/TextFileProvider/oneOf/2"></a>_object_
    _[RelativeFile](#definitions/RelativeFile)_.
    - <a id="definitions/TextFileProvider/oneOf/2/properties/kind"></a>**`kind`** _(string,
      required)_: Must be: `"relative_path"`.
  - <a id="definitions/TextFileProvider/oneOf/3"></a>_object_
    _[RequiredFile](#definitions/RequiredFile)_.
    - <a id="definitions/TextFileProvider/oneOf/3/properties/kind"></a>**`kind`** _(string,
      required)_: Must be: `"required"`.

#### <a id="definitions/Templatable%20boolean"></a>Templatable boolean

A templatable boolean that can be replaced with a user specified value at runtime.

- **One of**
  - <a id="definitions/Templatable%20boolean/oneOf/0"></a>_string_: The value that should be
    templated. Must match pattern: `^\{\{ \w+ \}\}$`
  - <a id="definitions/Templatable%20boolean/oneOf/1"></a>_boolean_: Statically provided data.

#### <a id="definitions/Templatable%20integer"></a>Templatable integer

A templatable integer that can be replaced with a user specified value at runtime.

- **One of**
  - <a id="definitions/Templatable%20integer/oneOf/0"></a>_string_: The value that should be
    templated. Must match pattern: `^\{\{ \w+ \}\}$`
  - <a id="definitions/Templatable%20integer/oneOf/1"></a>_integer_: Statically provided data.

#### <a id="definitions/Templatable%20string"></a>Templatable string

A templatable string that can be replaced with a user specified value at runtime.

- **One of**
  - <a id="definitions/Templatable%20string/oneOf/0"></a>_string_: The value that should be
    templated. Must match pattern: `^\{\{ \w+ \}\}$`
  - <a id="definitions/Templatable%20string/oneOf/1"></a>_string_: Statically provided data.
