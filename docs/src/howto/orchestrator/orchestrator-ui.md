<!-- diataxis-type: howto -->

# Working with the Orchestrator UI

The [Orchestrator UI][0] is the primary way to interact with the Orchestrator. From it you can
monitor and cancel Test Runs, inspect their results, trigger new runs, and manage your
[Known Test Plans][1]. The `rtf` CLI is only needed for triggering runs from scripts and for pulling
run output to your local machine.

The UI uses the same Google sign-in as the rest of the Orchestrator (see
[Getting set up with the Orchestrator][2]). Once signed in, a badge in the header shows your role
for the page you are viewing: **Admin**, **Owner**, or **Can trigger**.

## Finding a run

There are three ways to find a run in the UI:

- **Recent runs**: the home page lists recent Test Runs, which can be filtered by who started them
  and how recently they were started.
- **By ID**: enter a Test Run ID in the box in the header and select **View status**.
- **By Known Test Plan**: open the Known Test Plan from **Registered Test Plans** and look under
  **Previous runs**.

Runs triggered using `rtf remote run` or `rtf remote run-known` also print a direct link to the new
run's page.

## Checking queue and cluster capacity

The home page shows the current state of the Orchestrator:

- **Current Queue State**: queued and running Test Executions compared to the Orchestrator's total
  capacity
- **Workload Pools**: queued work for each [Workload Pool][4], along with the utilization of each
  cluster and any [Dedicated Cluster][5] claims
- **Recent Executions**: the number of Test Executions run over the last 24 hours
- **Workload pool configuration**: the rate limits, clusters, and nodes for each pool

If the Orchestrator is busy, your run will remain queued until capacity becomes available. Check
these sections before triggering particularly large runs.

## Inspecting a run

A run's page refreshes automatically while the run is in progress. It shows the run's overall
status, the Trigger Variables it was started with, and a breakdown of its Test Executions by status.
The list of executions can be filtered by status, and each execution provides links to:

- The execution's own page, showing its exit code and status history
- The container logs for the execution in GCP
- The execution's metrics in Grafana
- The execution's `log.txt` and `output.zip`

To download the output for every execution in the run at once, select **Download full run output**
and run the `rtf` command that it shows.

Execution output is only [retained for a limited time][6], so these downloads stop working for older
runs.

## Cancelling and re-running

To cancel a run, select **Cancel run** on the run's page and confirm. This button is only shown on
runs that you started or that belong to a Known Test Plan that you own. Admins are able to cancel
any run.

To run a Known Test Plan again with the same variables, select **Re-run**. This opens the Known Test
Plan's trigger form, pre-filled with the variables from the original run.

## Triggering a run

To trigger a Known Test Plan, open its page and use the **Trigger a run** section. The form starts
from the Test Plan's default variable values on the repository's default branch. If you aren't
permitted to trigger the Known Test Plan, the page lists its owners for you to contact.

To run from a different branch, tag, or commit, enter it under **Ref** and select **Load ref**. The
variables and defaults shown in the form are then read from that ref, so any variables that only
exist on your branch are available to change.

To override a Trigger Variable, select **+ Change a variable** and pick it from the list. Adding
several values for a variable runs it as a matrix dimension, and **Compound dimensions** let you
list combinations of variables that should vary together. Every variable that you have changed is
shown under **Changed variables**, and the remainder are listed under **Using test plan defaults**.

The variables that will be submitted are shown under **Raw variables JSON**. This can be edited
directly, which is useful for pasting in variables from elsewhere: select **Apply JSON to form** to
load them into the form.

Once you are happy with the form, select **Preview executions** to check the Test Executions that
will be run, and then **Trigger _N_ executions** to start the run.

### Using Registered Variables

If the Known Test Plan has [Registered Variables][7], select them from **Start from** to load them
into the form. You can then make further changes before triggering the run: while the form differs
from the Registered Variables a **modified** marker is shown, and selecting **reset** restores the
registered values. A run triggered from the UI uses the values in the form at the time it was
triggered, so later changes to the Registered Variables don't affect it.

**Manage** lists every set of Registered Variables for the Known Test Plan, and lets you load any of
them into the form.

Owners and Admins are also able to modify Registered Variables:

- To save the current form as new Registered Variables, select **Register variables…** and give them
  a name.
- To change existing Registered Variables, load them using **Start from**, make your changes, and
  select **Update "_name_"**.
- To delete Registered Variables, select **Delete** next to them in **Manage**.

Changes to Registered Variables apply to everyone using the Known Test Plan, so the UI asks you to
confirm before overwriting or deleting them.

To trigger a Test Plan that hasn't been registered, select **Trigger from GitHub** and provide the
organization, repository, and path to the `test-plan.yaml`, along with an optional Git ref and any
variables as a JSON object. Matrix dimensions are provided as JSON arrays. Runs triggered this way
always run in the default Workload Pool.

## Editing a Known Test Plan

Owners and Admins can edit a Known Test Plan from the **Edit test plan** section of its page. Fields
that can only be changed by an Admin are disabled for owners.

See [Registering and managing Known Test Plans][3] for what each setting does.

[0]: https://api.rtf.apollographql.com/ui
[1]: ../../reference/glossary.md#known-test-plan
[2]: getting-set-up.md
[3]: known-test-plans.md
[4]: ../../reference/glossary.md#workload-pool
[5]: ../../reference/glossary.md#dedicated-cluster
[6]: ../../reference/orchestrator/limits-permissions-and-concepts.md#output-storage
[7]: ../../reference/glossary.md#registered-variables
