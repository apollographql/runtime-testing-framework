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

- A unique name and an optional description for the Known Test Plan
- The GitHub organization, repository, and path to your `test-plan.yaml`
- The email addresses of the owners of the Known Test Plan

Once registered, your Known Test Plan will be listed under **Registered Test Plans** in the
Orchestrator UI.

## Configuring your Known Test Plan

Owners of a Known Test Plan are able to edit its configuration from the **Edit test plan** section
of its page in the Orchestrator UI. As an owner you are able to:

- Update the name, description, and GitHub location of the Known Test Plan
- Add or remove **Trigger users**
- Restrict **Triggers** so that only owners and trigger users can trigger runs
- Disable **Automated triggers** to prevent the Known Test Plan from being run from CI
- Enable or disable **Namespace write access** for your Scenario
- Set **Node label weights** for runs on a [Dedicated Cluster][9]

Adding or removing owners, pinning the Known Test Plan to a Workload Pool, and requiring a Dedicated
Cluster can only be done by an Admin. To request these changes, start a conversation in
[#proj-runtime-testing-framework][3]. See [Orchestrator internal processes][4] for other changes
that need an Admin.

See [Known Test Plan configuration][5] for details of each setting.

## Registering variables for your Known Test Plan

The Orchestrator allows owners to save sets of standard trigger variables against Known Test Plans
as [Registered Variables][10]. Registered Variables are visible to everyone who can view the Known
Test Plan, and can be used as the starting point for a run from the Orchestrator UI, the `rtf` CLI,
and CI.

To register a new variable set, set up the trigger form on the Known Test Plan's page with the
values you want and select **Register variables…**. See [Using Registered Variables][11] for how to
update and delete them.

Only owners and Admins are able to create, update, or delete Registered Variables.

## Triggering your Known Test Plan

The simplest way to trigger a run is from the **Trigger a run** section of the Known Test Plan's
page in the Orchestrator UI. From there you can trigger a run using the Test Plan's default variable
values, start from a set of Registered Variables, run from a different branch, tag, or commit, and
override individual Trigger Variables. See [Triggering a run][12] for details.

To trigger a run from a script or an agent, use the `rtf` CLI:

```bash
rtf remote run-known <test-plan-uuid>
rtf remote ci-run-known <test-plan-uuid> --ref <git-ref> --var <key>=<value>
rtf remote ci-run-known <test-plan-uuid> --registered-vars <name> --var <key>=<value>
```

The `--registered-vars` flag uses the named Registered Variables as the base for the run, with any
variables provided using `--var` or `--vars` taking precedence. Registered Variables are looked up
when the run is triggered, so the run always uses their latest values. See [Using `rtf remote`][13]
for how the two are combined.

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
[10]: ../../reference/glossary.md#registered-variables
[11]: orchestrator-ui.md#using-registered-variables
[12]: orchestrator-ui.md#triggering-a-run
[13]: ../../reference/orchestrator/rtf-remote.md#triggering-runs
