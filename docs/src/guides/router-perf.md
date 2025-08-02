# Router Performance Testing

> **Note**: The `router-scale` example contained in this guide is stored in the [rtf-morgue][0].
> This contains many examples of how to run different styles of tests in RTF. We recommend looking
> through the examples in the morgue to find a test plan that matches your use cases(s) once
> familiar with the steps in this guide.

### Table of contents

- [Overview](#overview)
- [Setting up access](#gcp-access)
  - [GCP](#gcp-access)
  - [Studio API](#studio-api-access)
  - [GitHub](#github-access)
  - [Clone rtf-morgue](#clone-rtf-morgue)
- [Replacing placeholder values](#replacing-placeholder-values)
- [Executing the test plan](#executing-the-test-plan)

## Overview

The test plans found in [rtf-morgue/test-plans/router-scale][1] demonstrate how to run Router
performance tests in a way similar to the existing [router-scale][2] testing tool.

Rather than running as a single test plan as we did in [hello world](./hello-world.md), these tests
are run using a _pair_ of test plans:

- The first is a [wrapper][3] that is used to spin up an ephemeral GCP VM where the tests will be
  executed. This test plan is decoupled from the actual test you are running and simply provides a
  shared orchestration layer for provisioning VMs and setting them up to be able to run an `rtf`
  test plan which is rsync'd across for execution on the VM rather than locally on your laptop (or
  directly in CI).
- The second is the test plan that we will actually execute on the VM as the test itself. For the
  purposes of this guide we'll be using the [router-perf][4] test plan that will spin up a build of
  the Apollo [Router][5] with mocked subgraphs and run a simple performance test against it using
  [vegeta][6].

As with the "hello, world!" test plan you will need to have the `rtf` binary installed. Please see
the details found in the [Getting Started](./index.md) page for how to get set up if you have not
done so already.

Unlike the "hello, world!" example, these test plans requires some additional setup and access to
resources which will need to be in place before things will work, so lets sort that out first.

## GCP Access

You can request access to GCP by using the `/assist` command in Slack to open a ticket with IT. Once
your access request has been actioned, navigate to the `router-performance` project’s [IAM page][8]
and look for your team name in that list. If your team does not have the
`Service Account Token Creator` role then you will also need to request that using `/assist`.

> An example PR to handle this can be found [here][9],

Once that role has been granted to your team you will need to ensure that you have the
[gcloud CLI][10] installed and configured. This can be done by running `gcloud init` after
completing the installation instructions in the that link and then following the steps it provides
(for a fresh install) or by running `gcloud auth login` if you already have the CLI installed.

Finally you will also need to make sure the beta gcloud components are installed, which can be done
by running `gcloud components install beta`.

## Studio API Access

In order to run test plans using production data you will need to have elevated permissions in
Studio. This is handled using [SHERRIF][11] and unlike the GCP access steps above this will need to
be completed each time you want to run tests that use production data.

- Open a ticket with SHERIFF as instructed [here][12] or alternatively just go [here][13] directly.
- Select “Access Type: Apollo Admin Access”. You need “Read Only” access to the “Prod” environment.
- Once your admin access has been granted you can create a new personal Studio API key [here][14].
  Note that your elevated permissions are only valid for 12 hours and that you will need to create a
  new API key each time you request access via SHERIFF.
- Export your new API key as `APOLLO_KEY` using your preferred mechanism for managing shell
  environment variables before running rtf.
  - We use [mise][15] in the rtf repo and manage our environment variables in a `.env` file that is
    git ignored. We recommend managing your environment variables in a local `.env` file.

## GitHub Access

In order to run this test plan, you will need a `GITHUB_TOKEN`. This is so that the VM can pull the
RTF binary directly from the GitHub workflow artifacts. This is also required so that GitHub file
providers will run (if being used).

To create one, follow the instructions on creating a [personal access token (classic)][16]. The
token will only need `repo` scopes.

> **Warning**: Once created, you must use the `Configure SSO` option that appears next to your token
> in the Personal access tokens list. Make sure that the token is authenticated with the
> `apollographql` org. If this step is not completed then you will not authenticate successfully
> with the RTF repo.

Export your new API key as `GITHUB_TOKEN` using your preferred mechanism for managing shell
environment variables before running rtf.

## Clone rtf-morgue

Clone the [rtf-morgue][0] repository to your local machine. The commands below assume you are
running from the root of your local [rtf-morgue][0].

```bash
git clone https://github.com/apollographql/rtf-morgue.git
```

## Replacing placeholder values

Before either of the test plans can be checked and ran the placeholder values the contain need to be
filled in. Attempting to run them before doing this will deliberately fail checks so they can not be
run by accident.

### Wrapper

cd to the root of your checkout of the `rtf-morgue` repository and run the following command to
check the wrapper test plan:

```bash
rtf template test-plans/router-scale/wrapper/test-plan.yaml \
  --check
```

Doing so with a clean checkout of the repository should give you the following (expected) error
output:

```
ERROR (environment.setup.command.arg) invalid templating value: invalid value `1`, expected String
(environment.teardown.command.arg) unknown templating value: vm_name
(scenario.command_section.command.arg) unknown templating value: vm_name
(scenario.command_section.env_vars.RSYNC_DIR) invalid templating value: invalid value `1`, expected String
```

To fix this error we need to override the `vm_name_suffix` and `rsync_dir` values with a suffix for
the VM name and the absolute path to a local directory that needs to be synced to the VM.Since we
are running this test plan from the root of the repo we can do that simply by using `--value` flag:

```bash
rtf template test-plans/router-scale/wrapper/test-plan.yaml \
  --value "vm_name_suffix=suffix" \
  --value "rsync_dir=$(pwd)" \
  --check
```

> **Info**: The `rsync_dir` copies a full, local directory to the VM. This directory is expected to
> contain the test plan that will be executed on the VM and all of its dependencies (scripts, config
> files etc). In this example, we copy the full `rtf-morgue` repo. It will be present on the VM at
> `./rsync-dir`.

You will still get an error with this command. Expected error output:

```
ERROR (environment.teardown.command.arg) unknown templating value: vm_name
(scenario.command_section.command.arg) unknown templating value: vm_name
```

This is because the environment setup provides a `vm_name` (which is generated using the
`vm_name_suffix`). When running the test plan with the `run` subcommand you will not need to specify
this value. However, to check that a test plan fully templates, this value will need to be specified
with the `--value` flag too.

```bash
rtf template test-plans/router-scale/wrapper/test-plan.yaml \
  --value "vm_name_suffix=suffix" \
  --value "rsync_dir=$(pwd)" \
  --value "vm_name=name" \
  --check
```

Running this updated command should output the templated test plan in your terminal.

### Router-perf

Checking the router-perf test plan looks similar but with a few more additional values that will
come from the environment setup command when the test plan is run. Lets start by setting those:

```bash
rtf template test-plans/router-scale/performance/router-perf/test-plan.yaml \
  --value 'router_pid="1"' \
  --value 'router_cgroup="true"' \
  --check
```

Again, you should see some expected error output due to a placeholder value, this time the graph ref
that we are running the tests for:

```
ERROR (subgraphs) the provided string was not a valid graph ref: expected a string of the form 'graph_id@variant'
(supergraph.graphql) the provided string was not a valid graph ref: expected a string of the form 'graph_id@variant'
(canned_ops.json) the provided string was not a valid graph ref: expected a string of the form 'graph_id@variant'
```

> **NOTE**: If you also see errors around no API key being provided then you have not exported your
> API key under the `APOLLO_KEY` environment variable (see the
> [Studio API access](#studio-api-access) section above)

As before we can fix this error by using `--value` flag to override the placeholder value, this time
for the `graph_ref` value. If you are unsure of a graph to test against then a good starting point
is to pick one of the graphs found in the current [router-scale corpus][17]:

```bash
rtf template test-plans/router-scale/performance/router-perf/test-plan.yaml \
  --value 'router_pid="1"' \
  --value 'router_cgroup="true"' \
  --value "graph_ref=YOUR-CHOSEN@GRAPH" \
  --check
```

Running this updated command should output the templated test plan in your terminal.

## Executing the Test Plan

Now that both test plans check successfully you can use the wrapper test plan to execute a
performance test on an ephemeral VM. We don't need to override the values coming from the
environment setup any more but we _do_ still need to specify the vm name suffix and graph ref:

```bash
rtf run test-plans/router-scale/wrapper/test-plan.yaml \
  --value "vm_name_suffix=$(whoami)" \
  --value "rsync_dir=$(pwd)" \
  --value "graph_ref=YOUR-CHOSEN@GRAPH"
```

> **NOTE**: Instead of using the `--value` flag to specify each value individually, you can also use
> the `--values` (plural) flag to point to a JSON file containing multiple values:
>
> `rtf run my-test-plan.yaml --values my-values.json`

Once the `output` directory has been created you can run

```bash
tail -f output/vm-log.txt`
```

in another terminal window to follow the execution and then run

```bash
./test-plans/router-scale/wrapper/scripts/gcloud-ssh-wrapper.sh rtf-$(whoami)
```

once the VM is up to get an SSH session started on the VM if desired. The test results are copied to
your local RTF output directory before the test run completes. They should be stored in
`output/results`. If the VM is not deleted at the end of the test run the results are deleted from
the VM by the clean up script (so are only available locally).

To persist the VM between runs, override the `delete_vm` value in the wrapper test plan with
`"false"`. In practice, this could be relaced with any value that is not `"true"`, since that is the
only value being matched on in the `cleanup-router-scale-vm.sh` script:

```bash
rtf run test-plans/router-scale/wrapper/test-plan.yaml \
  --value "vm_name_suffix=$(whoami)" \
  --value "rsync_dir=$(pwd)" \
  --value "graph_ref=YOUR-CHOSEN@GRAPH" \
  --value 'delete_vm="false"'
```

To execute a different test plan in the morgue, the `test_plan_dir` value can be overridden. The
example below will run the `router-1234` regression test instead of `router-perf`.

```bash
rtf run test-plans/router-scale/wrapper/test-plan.yaml \
  --value "vm_name_suffix=$(whoami)" \
  --value "rsync_dir=$(pwd)" \
  --value "graph_ref=YOUR-CHOSEN@GRAPH" \
  --value "test_plan_dir=./rsync-dir/test-plans/router-scale/underprovisioned/router-1234"
```

The `rsync_dir` and `test_plan_dir` can be used in combination to run any local test plans that you
write and run them on the `router-scale` VM.

[0]: https://github.com/apollographql/rtf-morgue
[1]: https://github.com/apollographql/rtf-morgue/tree/main/test-plans/router-scale
[2]: https://github.com/apollographql/router-scale
[3]: https://github.com/apollographql/rtf-morgue/tree/main/test-plans/router-scale/wrapper
[4]: https://github.com/apollographql/rtf-morgue/tree/main/test-plans/router-scale/router-perf
[5]: https://github.com/apollographql/router
[6]: https://github.com/tsenart/vegeta
[7]: https://github.com/apollographql/runtime-testing-framework
[8]: https://console.cloud.google.com/iam-admin/iam?referrer=search&project=router-performance
[9]: https://github.com/mdg-private/platform-teams/pull/178
[10]: https://cloud.google.com/sdk/docs/install
[11]: https://apollographql.atlassian.net/wiki/spaces/SecOps/pages/805404674/SHERIFF
[12]: https://apollographql.atlassian.net/wiki/spaces/SecOps/pages/805568513
[13]: https://apollographql.atlassian.net/servicedesk/customer/portal/1/group/3/create/1247
[14]: https://studio.apollographql.com/user-settings/api-keys
[15]: https://apollographql.atlassian.net/wiki/spaces/Foundation/pages/1490583600/Mise+standardized+dev+tools+made+simple
[16]: https://docs.github.com/en/authentication/keeping-your-account-and-data-secure/managing-your-personal-access-tokens#creating-a-personal-access-token-classic
[17]: https://github.com/apollographql/router-scale/blob/main/data/router_2.0_launch/corpus.yaml#L7-L46
