<!-- diataxis-type: reference -->

# Using `rtf remote`

The `rtf remote` subcommands interact with the [RTF Orchestrator Service][0]. Most interactions with
the Orchestrator are best carried out using the [Orchestrator UI][1]: the `rtf` CLI is intended for
triggering runs from scripts and agents, and for pulling run output to the local machine. This page
summarizes each subcommand. For every flag, see the [CLI reference][2].

## Authentication

All subcommands except `prepare` authenticate with the Orchestrator through Google IAP and require:

- Access to the Orchestrator (see [Orchestrator internal processes][3])
- Application default credentials from `gcloud auth application-default login`

On first use, the CLI opens a browser window to complete sign-in and caches the resulting token.

## Environment variables

| Variable               | Description                                                          |
| ---------------------- | -------------------------------------------------------------------- |
| `RTF_ORCHESTRATOR_URL` | Orchestrator base URL. Defaults to the [production Orchestrator][4]. |
| `GITHUB_TOKEN`         | GitHub token used by `--github` to fetch the Test Plan locally.      |

## Triggering runs

| Subcommand     | Arguments                                                                       | Description                                                            |
| -------------- | ------------------------------------------------------------------------------- | ---------------------------------------------------------------------- |
| `run`          | `<test-plan-path>` `[--github]` `[--ref <ref>]`                                 | Triggers a Test Run. Prints the Test Run ID and a UI link.             |
| `ci-run`       | `<test-plan-path>` `[--github]` `[--ref <ref>]` `[--poll-interval-seconds <n>]` | Triggers a Test Run and polls until it completes.                      |
| `run-known`    | `<test-plan-uuid>` `[--ref <ref>]`                                              | Triggers a [Known Test Plan][9]. Prints the Test Run ID and a UI link. |
| `ci-run-known` | `<test-plan-uuid>` `[--ref <ref>]` `[--poll-interval-seconds <n>]`              | Triggers a Known Test Plan and polls until it completes.               |
| `prepare`      | `<test-plan-path>` `[--github]` `[--ref <ref>]`                                 | Writes the [Trigger Payload][10] for a Test Plan to stdout.            |

- `--var <key>=<value>` and `--vars <file.json>` set Test Plan variables for these subcommands.
- `--github` reads `<test-plan-path>` as `<org>/<repo>/<path>` and fetches the Test Plan from GitHub
  on the local machine before submitting it.
- For `run-known` and `ci-run-known`, `--ref` overrides the Known Test Plan's default branch.
- `--poll-interval-seconds` defaults to `10`.
- Runs triggered with `run` and `ci-run` always execute in the default [Workload Pool][11].

`ci-run` and `ci-run-known` exit with:

| Exit code | Meaning                                 |
| --------- | --------------------------------------- |
| `0`       | All executions succeeded                |
| `1`       | At least one execution failed           |
| `2`       | The run was unrunnable or was cancelled |

## Pulling output

Output is only [retained for a limited time][13]; once it has expired, the commands that download or
print it fail for the affected executions.

| Subcommand         | Arguments                                       | Description                                                                 |
| ------------------ | ----------------------------------------------- | --------------------------------------------------------------------------- |
| `run-output`       | `<run-id>` `[--outdir <dir>]` `[--force]`       | Downloads the run summary, and each execution's `log.txt` and `output.zip`. |
| `execution-output` | `<execution-id>` `[--outdir <dir>]` `[--force]` | Downloads an execution's summary, `log.txt`, and `output.zip`.              |
| `execution-log`    | `<execution-id>`                                | Prints an execution's `log.txt`.                                            |
| `run-status`       | `<run-id>` `[--with-executions]`                | Prints the [Test Run Summary][12] as JSON.                                  |
| `execution-status` | `<execution-id>`                                | Prints a Test Execution summary as JSON.                                    |

`--outdir` defaults to `output`. `--force` removes an existing output directory first.

`run-output` writes:

```text
<outdir>/
  run-summary.json
  <execution-name>/
    log.txt
    output.zip
```

## Cancelling a run

Runs are normally cancelled from their page in the Orchestrator UI. To cancel a run from the CLI,
use `rtf remote request`:

```bash
rtf remote request -X POST -d '{"id": "<run-id>"}' admin/purge-run
```

See [Limits, permissions, and concepts][5] for who is able to cancel a run.

## Related pages

- [Getting set up with the Orchestrator][6]
- [Working with the Orchestrator UI][7]
- [Running with the Orchestrator][8] tutorial

[0]: ../glossary.md#rtf-orchestrator-service
[1]: https://api.rtf.apollographql.com/ui
[2]: ../cli-help.md
[3]: internal-processes.md
[4]: https://api.rtf.apollographql.com
[5]: limits-permissions-and-concepts.md#permissions
[6]: ../../howto/orchestrator/getting-set-up.md
[7]: ../../howto/orchestrator/orchestrator-ui.md
[8]: ../../tutorials/running-with-the-orchestrator/index.md
[9]: ../glossary.md#known-test-plan
[10]: ../glossary.md#trigger-payload
[11]: ../glossary.md#workload-pool
[12]: ../glossary.md#test-run-summary
[13]: limits-permissions-and-concepts.md#output-storage
