<!-- diataxis-type: howto -->

# Running test plans in GitHub Actions

RTF provides a set of reusable GitHub Actions in the [release-tooling][0] repository for running and
validating test plans in CI. These actions handle authenticating with internal registries and
pulling the `rtf-toolbox` Docker image so you don't need to install the RTF CLI on the runner.

> All actions require the job to have `id-token: write` permission for GCP workload identity
> federation.

## Running a test plan

**Problem**: You want to run a test plan as part of your CI pipeline.

**Solution**: Use the `rtf-run-test-plan` action:

```yaml
jobs:
  e2e:
    runs-on: ubuntu-latest
    permissions:
      contents: read
      id-token: write
    steps:
      - uses: actions/checkout@v6

      - name: Run smoke tests
        uses: apollographql/release-tooling/rtf-run-test-plan@main
        with:
          test_plan_path: test-plans/smoke/test-plan.yaml
          github_token: ${{ github.token }}
```

**Discussion**: The action accepts the following inputs:

| Input | Required | Description |
|---|---|---|
| `test_plan_path` | yes | Path to the test plan file |
| `vars_file` | no | Path to a JSON file containing template variables |
| `verbose` | no | Set to `"true"` for TRACE level logging (default is INFO) |
| `github_token` | no | GitHub token for accessing private repositories |
| `apollo_key` | no | Apollo API key for GraphOS providers |
| `apollo_sudo` | no | Enable Apollo sudo mode for elevated GraphOS permissions |

If your test plan references files from private GitHub repositories using [github file providers][1],
you must pass `github_token`.

## Overriding variables in CI

**Problem**: You want to run a test plan with different variable values in CI than the defaults
(e.g. testing a locally built Docker image instead of a published one).

**Solution**: Create a separate variables file for CI and pass it via `vars_file`:

```json
{
  "mcp_server_image": "my-service",
  "mcp_server_tag": "local"
}
```

```yaml
      - name: Build Docker image
        run: docker build -t my-service:local .

      - name: Run tests
        uses: apollographql/release-tooling/rtf-run-test-plan@main
        with:
          test_plan_path: e2e/smoke/test-plan.yaml
          vars_file: e2e/smoke/ci-variables.json
          github_token: ${{ github.token }}
```

**Discussion**: Variables passed via `vars_file` override both the test plan's `variables` section
and any defaults in the environment or scenario configs. The test plan's defaults still serve as the
configuration for running locally during development.

## Validating test plans in CI

**Problem**: You want to check that all test plans in a directory template correctly on every PR.

**Solution**: Use the `rtf-validate-test-plans` action:

```yaml
jobs:
  validate:
    runs-on: ubuntu-latest
    permissions:
      contents: read
      id-token: write
    steps:
      - uses: actions/checkout@v6

      - name: Validate test plans
        uses: apollographql/release-tooling/rtf-validate-test-plans@main
        with:
          directories: test-plans
```

**Discussion**: This action finds all `test-plan.yaml` files in the specified directories and runs
`rtf template --check` against each one using its companion `template-variables.json`. You can
customise the file names with the `test_plan_path` and `variables_path` inputs. Multiple directories
can be specified as a comma-separated list.

## Testing custom providers in CI

**Problem**: You want to verify that custom providers produce the expected output on every PR.

**Solution**: Use the `rtf-test-custom-providers` action:

```yaml
      - name: Test custom providers
        uses: apollographql/release-tooling/rtf-test-custom-providers@main
        with:
          directories: lib/custom-providers
```

**Discussion**: This action discovers custom provider definitions (`provider.yaml` by default) in
the specified directories and runs their test cases. See [writing custom providers][2] for how to
add test cases to your providers.

[0]: https://github.com/apollographql/release-tooling
[1]: ../reference/framework/file-providers.md#github-file
[2]: ../tutorials/custom-providers/index.md
