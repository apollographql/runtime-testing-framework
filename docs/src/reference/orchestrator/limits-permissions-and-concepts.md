<!-- diataxis-type: reference -->

# Limits, permissions, and concepts

This page describes Orchestrator behavior that isn't visible from the `rtf` CLI: identities,
permissions, rate limits, statuses, and output storage. See [Workload Pools][3] for how runs are
scheduled.

## Identities

The Orchestrator identifies each user from their Google sign-in:

| Identity             | Description                                                                                    |
| -------------------- | ---------------------------------------------------------------------------------------------- |
| User                 | Any Apollo user with access to the Orchestrator.                                               |
| Admin                | A member of the Runtime Readiness team who administers the Orchestrator.                       |
| [Automation User][0] | A CI workflow using the Orchestrator GitHub Action, identified separately for each repository. |

Users can also hold a role on individual [Known Test Plans][7]:

| Role        | Description                                                                      |
| ----------- | -------------------------------------------------------------------------------- |
| Owner       | Can configure the Known Test Plan and cancel any of its runs.                    |
| Can trigger | Can trigger the Known Test Plan when its triggers are restricted to named users. |

The badge in the header of the [Orchestrator UI][1] shows your identity and your role on the Known
Test Plan you are viewing.

## Permissions

| Action                                   | Admin | Owner | Can trigger | User                       | Automation User                    |
| ---------------------------------------- | ----- | ----- | ----------- | -------------------------- | ---------------------------------- |
| Trigger an unregistered Test Plan        | Yes   | Yes   | Yes         | Yes                        | Not supported                      |
| Trigger a Known Test Plan                | Yes   | Yes   | Yes         | Unless triggers restricted | Unless automated triggers disabled |
| Register a Known Test Plan               | Yes   | No    | No          | No                         | No                                 |
| Edit a Known Test Plan                   | Yes   | Yes¹  | No          | No                         | No                                 |
| Add or remove owners                     | Yes   | No    | No          | No                         | No                                 |
| Cancel a run                             | Yes   | Yes²  | Own runs    | Own runs                   | No                                 |
| View runs, executions, output, and plans | Yes   | Yes   | Yes         | Yes                        | Yes                                |

1. Except the pinned Workload Pool, the Dedicated Cluster setting, and owners.
2. Runs of Known Test Plans they own, and runs they started.

## Rate limits

Rate limits are configured per [Workload Pool][8] and are applied to each user separately.
Automation Users are limited separately for each repository. Admins are exempt. Each pool limits the
number of:

- Runs in progress at once
- Runs waiting to start
- Test Executions waiting to start
- Runs triggered in the last hour

The current limits for each pool are shown under **Workload pool configuration** in the Orchestrator
UI, and are set in the [Orchestrator configuration file][2].

Triggering a run that would exceed one of these limits fails with a `429 Too Many Requests` error
naming the pool and the limit that was exceeded. The Orchestrator also has an overall limit on
queued Test Executions: when the queue is full, triggers fail with a `503 Service Unavailable` error
until capacity frees up.

## Statuses

Test Runs and Test Executions progress through these statuses:

| Status              | Terminal | Description                                                         |
| ------------------- | -------- | ------------------------------------------------------------------- |
| `INITIALISING`      | No       | Accepted and queued                                                 |
| `RESOLVING`         | No       | Resolving the Environment configuration                             |
| `PROVISIONING`      | No       | Creating the namespace and deploying services                       |
| `ENVIRONMENT_READY` | No       | Services are healthy and the Scenario is starting                   |
| `RUNNING`           | No       | The Scenario is running                                             |
| `SUCCESSFUL`        | Yes      | The Scenario exited with code `0`                                   |
| `FAILED`            | Yes      | The Scenario exited with a non-zero code                            |
| `UNRUNNABLE`        | Yes      | The execution couldn't complete, for example due to a failed deploy |
| `CANCELLED`         | Yes      | The run was cancelled by a user                                     |

## Output storage

Each Test Execution's `log.txt` and `output.zip` are stored in Google Cloud Storage and are
available once the execution has finished. Downloads from the Orchestrator UI and the `rtf` CLI are
served using time-limited signed URLs.

Stored output is retained for 14 days. A [lifecycle policy][9] on the bucket deletes each object 14
days after it was uploaded, after which the output can no longer be downloaded. Download any output
you need to keep before then.

## Limitations

- Known Test Plans must template without any Trigger Variable overrides.
- The Orchestrator UI only shows details for Known Test Plans on their repository's default branch.
- Each Test Execution runs a single Scenario pod.

## Related pages

- [Workload Pools][3]
- [Using `rtf remote`][4]
- [Known Test Plan configuration][5]
- [Orchestrator internal processes][6]

[0]: ../glossary.md#automation-user
[1]: https://api.rtf.apollographql.com/ui
[2]: https://github.com/mdg-private/kanaveral/blob/main/management-cluster/orchestrator/orchestrator-config.yaml
[3]: workload-pools.md
[4]: rtf-remote.md
[5]: known-test-plan-configuration.md
[6]: internal-processes.md
[7]: ../glossary.md#known-test-plan
[8]: ../glossary.md#workload-pool
[9]: https://github.com/mdg-private/runtime-readiness-terraform/blob/main/projects/runtime-testing-framework/gcs.tf
