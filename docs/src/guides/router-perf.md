# Router Performance Testing

> **Note**: The `router-scale` example contained in RTF repo is intended as an example to
help you get started. For the most up-to-date `router-scale` scripts and other example
test plans please go to the [rtf-morgue][0].

### Table of contents
  - [Overview](#overview)
  - [Setting up access](#gcp-access)
    - [GCP](#gcp-access)
    - [Studio API](#studio-api-access)
  - [Replacing placeholder values](#replacing-placeholder-values)
  - [Executing the Test Plan](#executing-the-test-plan)


## Overview

The Test Plans found in [example-test-plans/router-scale][1] demonstrate how to
run Router performance tests in a way similar to the existing [router-scale][2]
testing tool.

Rather than running as a single Test Plan as we did in [hello world](./hello-world.md),
these tests are run using a _pair_ of Test Plans:
  - The first is a [wrapper][3] that is used to spin up an ephemeral GCP VM
    where the tests will be executed. This Test Plan is decoupled from the actual
    test you are running and simply provides a shared orchestration layer for
    provisioning VMs and setting them up to be able to run an `rtf` Test Plan which
    is rsync'd across for execution on the VM rather than locally on your laptop
    (or directly in CI).
  - The second is the Test Plan that we will actually execute on the VM as the
    test itself. For the purposes of this guide we'll be using the [router-perf][4]
    Test Plan that will spin up a build of the Apollo [Router][5] with mocked
    subgraphs and run a simple performance test against it using [vegeta][6].

As with the "hello, world!" Test Plan you will need a local checkout of the
[runtime-testing-framework][7] repository and you will need to have the `rtf` binary
installed. Please see the details found in the [Getting Started](./index.md) page for
how to get set up if you have not done so already.

Unlike the "hello, world!" example, these Test Plans requires some additional setup and
access to resources which will need to be in place before things will work, so lets sort
that out first.


## GCP Access

You can request access to GCP by using the `/assist` command in Slack to open a
ticket with IT. Once you’re access request has been actioned, navigate to the
`router-performance` project’s IAM page [here][8] and look for your team name
in that list. If your team does not have the `Service Account Token Creator`
role then you will also need to request that using `/assist`.

> An example PR to handle this can be found [here][9],

Once that role has been granted to your team you will need to ensure that you
have the [gcloud CLI][10] installed and configured. This can be done by running
`gcloud init` after completing the installation instructions in the that link
and then following the steps it provides (for a fresh install) or by running
`gcloud auth login` if you already have the CLI installed.

Finally you will also need to make sure the beta gcloud components are installed,
which can be done by running `gcloud components install beta`.

## Studio API Access
In order to run test plans using production data you will need to have elevated
permissions in Studio. This is handled using [SHERRIF][11] and unlike the GCP
access steps above this will need to be completed each time you want to run
tests that use production data.

- Open a ticket with SHERIFF as instructed [here][12] or alternatively just go
  [here][13] directly.
- Select “Access Type: Apollo Admin Access”. You need “Read Only” access to the
  “Prod” environment.
- Once your admin access has been granted you can create a new personal Studio
  API key [here][14]. Note that your elevated permissions are only valid for 12
  hours and that you will need to create a new API key each time you request
  access via SHERIFF.
- Export your new API key as `APOLLO_KEY` using your perferred mechanism for
  managing shell environment variables before running rtf.
  - [direnv][15] is a nice way to do this if you don't have an existing setup
    you are already using.


## Replacing placeholder values

Before either of the Test Plans can be checked and ran the placeholder values
the contain need to be filled in. Attempting to run them before doing this will
deliberately fail checks so they can not be run by accident.

### Wrapper
cd to the root of your checkout of the `runtime-testing-framework` repository
and run the following command to check the wrapper test plan:
```bash
rtf template example-test-plans/router-scale/wrapper/test-plan.yaml \
  --check
```

Doing so with a clean checkout of the repository should give you the following
(expected) error output:
```
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.37s
     Running `target/debug/rtf template example-test-plans/router-scale/wrapper/test-plan.yaml --check`
 INFO loading and resolving test plan
 INFO checking if templating will work
 INFO applying values
ERROR (environment.setup.command.arg) invalid templating value: invalid value `1`, expected String
(environment.teardown.command.arg) invalid templating value: invalid value `1`, expected String
(scenario.command_section.command.arg) invalid templating value: invalid value `1`, expected String
(scenario.command_section.env_vars.RTF_DIR) invalid templating value: invalid value `1`, expected String
```

To fix this error we need to override the `abs_path_rtf_repo` and `vm_name_suffix` values with the
absolute path of the rtf repo and a suffix for the VM name. Since we are running this test plan from
the root of the repo we can do that simply by using `--value` flag:
```bash
rtf template example-test-plans/router-scale/wrapper/test-plan.yaml \
  --value "abs_path_rtf_repo=$(pwd)" \
  --value "vm_name_suffix=suffix" \
  --check
```

You will still get an error with this command. Expected error output:
```
ERROR (environment.teardown.command.arg) unknown templating value: vm_name
(scenario.command_section.command.arg) unknown templating value: vm_name
```

This is because the environment setup provides a `vm_name` (which is generated using the `vm_name_suffix`).
When running the test plan with the `run` subcommand you will not need to specify this value. However, to
check that a test plan fully templates, this value will need to be specified with the `--value` flag too.
```bash
rtf template example-test-plans/router-scale/wrapper/test-plan.yaml \
  --value "abs_path_rtf_repo=$(pwd)" \
  --value "vm_name_suffix=suffix" \
  --value "vm_name=name" \
  --check
```

Running this updated command should output the templated test plan in your
terminal.


### Router-perf
Checking the router-perf test plan looks similar but with a few more additional
values that will come from the environment setup command when the test plan is
run. Lets start by setting those:
```bash
rtf template example-test-plans/router-scale/router-perf/test-plan.yaml \
  --value "router_pid=1" \
  --value "router_cgroup=true" \
  --check
```

Again, you should see some expected error output due to a placeholder value,
this time the graph ref that we are running the tests for:
```
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.21s
     Running `target/debug/rtf template example-test-plans/router-scale/router-perf/test-plan.yaml --check '--values={ "router_pid": "1", "router_cgroup": "true" }'`
 INFO loading and resolving test plan
 INFO checking if templating will work
 INFO applying values
 INFO checking test plan
ERROR (subgraphs) the provided string was not a valid graph ref expected a string of the form 'graph_id@variant'
(supergraph.graphql) the provided string was not a valid graph ref expected a string of the form 'graph_id@variant'
(canned_ops.json) the provided string was not a valid graph ref expected a string of the form 'graph_id@variant'
```

> **NOTE**: If you also see errors around no API key being provided then you
> have not exported your API key under the `APOLLO_KEY` environment variable
> (see the [Studio API access](#studio-api-access) section above)

As before we can fix this error by using `--value` flag to override the placeholder
value, this time for the `graph_ref` value. If you are unsure of a graph to test
against then a good starting point is to pick one of the graphs found in the current
[router-scale corpus][16]:
```bash
rtf template example-test-plans/router-scale/router-perf/test-plan.yaml \
  --value "router_pid='1'" \
  --value "router_cgroup='true'" \
  --value "graph_ref=YOUR-CHOSEN@GRAPH" \
  --check
```

Running this updated command should output the templated test plan in your
terminal.


## Executing the Test Plan

Now that both Test Plans check successfully you can use the wrapper Test Plan to execute
a performance test on an ephemeral VM. We don't need to override the values coming from
the environment setup any more but we _do_ still need to specify the repo location, vm
name suffix and graph ref:

> **NOTE**: Replace `YOUR_SUFFIX` with your actual suffix before running.
> We recommend using your name to avoid conflicts with any other users.

```bash
rtf run example-test-plans/router-scale/wrapper/test-plan.yaml \
  --value "abs_path_rtf_repo=$(pwd)" \
  --value "vm_name_suffix=YOUR_SUFFIX" \
  --value "graph_ref=YOUR-CHOSEN@GRAPH"
```

> **NOTE**: Instead of using the `--value` flag to specify each value individually,
> you can also use the `--values` (plural) flag to point to a JSON file containing
> multiple values:
> 
> `rtf run my-test-plan.yaml --values my-values.json`

Once the `output` directory has been created you can run `tail -f output/vm-log.txt`
in another terminal window to follow the execution and then run
`./example-test-plans/router-scale/wrapper/scripts/gcloud-ssh-wrapper.sh rtf-router-scale`
once the VM is up to get an SSH session started on the VM if desired. The test results
are copied to your local RTF output directory before the test run completes. They should
be stored in `output/results`. If the VM is not deleted at the end of the test run the
results are deleted from the VM by the clean up script (so are only available locally).

To persist the VM between runs, override the `delete_vm` value in the wrapper test plan
with `"false"`. In practice, this could be relaced with any value that is not `"true"`, 
since that is the only value being matched on in the `cleanup-router-scale-vm.sh` script:
```bash
rtf run example-test-plans/router-scale/wrapper/test-plan.yaml \
  --value "abs_path_rtf_repo=$(pwd)" \
  --value "vm_name_suffix=YOUR_SUFFIX" \
  --value "graph_ref=YOUR-CHOSEN@GRAPH" \
  --value 'delete_vm="false"'
```

  [0]: https://github.com/apollographql/rtf-morgue
  [1]: https://github.com/apollographql/runtime-testing-framework/tree/main/example-test-plans/router-scale
  [2]: https://github.com/apollographql/router-scale
  [3]: https://github.com/apollographql/runtime-testing-framework/tree/main/example-test-plans/router-scale/wrapper
  [4]: https://github.com/apollographql/runtime-testing-framework/tree/main/example-test-plans/router-scale/router-perf
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
  [15]: https://github.com/direnv/direnv
  [16]: https://github.com/apollographql/router-scale/blob/main/data/router_2.0_launch/corpus.yaml#L7-L46
