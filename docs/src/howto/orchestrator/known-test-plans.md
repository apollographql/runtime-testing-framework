<!-- diataxis-type: howto -->

# Registering and managing Known Test Plans

> **Note**: This page describes Apollo internal processes. Concrete details are maintained in
> internal Confluence documentation linked below.

A [Known Test Plan][0] is a Test Plan stored in GitHub that has been registered with the
Orchestrator. Registering a Test Plan allows it to be triggered from the [Orchestrator UI][1] and
from CI, gives you control over who is able to trigger it, and allows it to be scheduled outside of
the default [Workload Pool][7]. This guide covers registering a Known Test Plan, configuring it, and
triggering runs.

> **Prerequisites**
>
> - Access to the Orchestrator (see [Getting set up with the Orchestrator][2])
> - A Test Plan committed to a GitHub repository that the Orchestrator is able to read

## Making your Test Plan registrable

The Orchestrator reads Known Test Plans directly from GitHub each time they are triggered, so it
needs to be able to template them without any user supplied [Trigger Variables][8]:

1. Give every variable in your Test Plan a placeholder default value so that it templates without
   any overrides.
2. Make sure every `${...}` name in `matrix.variant_names` refers to a variable that is defined in
   the Test Plan.
3. Check that the Test Plan templates correctly when pulled from GitHub:

   ```bash
   rtf template --github <org>/<repo>/<path> --check
   ```

4. Merge the Test Plan to your repository's default branch. The Orchestrator UI is only able to
   display details for Known Test Plans that are present on the default branch.

If the Orchestrator is unable to template your Test Plan, its page in the Orchestrator UI will show
an error describing the problem.

## Registering your Test Plan

Registering a Test Plan is an Admin action. To request it, start a conversation in
[#proj-runtime-testing-framework][3]. You will need to provide:

- a unique name and an optional description for the Known Test Plan
- the GitHub organization, repository, and path to your `test-plan.yaml`
- the email addresses of the owners of the Known Test Plan

Once registered, your Known Test Plan will be listed under **Registered Test Plans** in the
Orchestrator UI.

## Configuring your Known Test Plan

Owners of a Known Test Plan are able to edit its configuration from the **Edit test plan** section
of its page in the Orchestrator UI. As an owner you are able to:

- update the name, description, and GitHub location of the Known Test Plan
- add or remove **Trigger users**
- restrict **Triggers** so that only owners and trigger users can trigger runs
- disable **Automated triggers** to prevent the Known Test Plan from being run from CI
- enable or disable **Namespace write access** for your Scenario
- set **Node label weights** for runs on a [Dedicated Cluster][9]

Adding or removing owners, pinning the Known Test Plan to a Workload Pool, and requiring a Dedicated
Cluster can only be done by an Admin. To request these changes, start a conversation in
[#proj-runtime-testing-framework][3]. See [Orchestrator internal processes][4] for other changes
that need an Admin.

See [Known Test Plan configuration][5] for details of each setting.

## Triggering your Known Test Plan

The simplest way to trigger a run is from the **Trigger a run** section of the Known Test Plan's
page in the Orchestrator UI. From there you can either trigger a run using the current variable
values, or use **Customize ref & variables** to run from a different branch, tag, or commit and to
override individual Trigger Variables.

To trigger a run from a script or an agent, use the `rtf` CLI:

```bash
rtf remote run-known <test-plan-uuid>
rtf remote ci-run-known <test-plan-uuid> --ref <git-ref> --var <key>=<value>
```

The UUID of your Known Test Plan is the final segment of the URL of its page in the Orchestrator UI.
To trigger your Known Test Plan from CI, see [Running from CI][6].

[0]: ../../reference/glossary.md#known-test-plan
[1]: https://api.rtf.apollographql.com/ui
[2]: getting-set-up.md
[3]: https://apollograph.slack.com/archives/C08UBLUD19C
[4]: ../../reference/orchestrator/internal-processes.md
[5]: ../../reference/orchestrator/known-test-plan-configuration.md
[6]: getting-set-up.md#running-from-ci
[7]: ../../reference/glossary.md#workload-pool
[8]: ../../reference/glossary.md#trigger-variables
[9]: ../../reference/glossary.md#dedicated-cluster
