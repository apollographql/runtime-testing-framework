# Router Performance Testing

### Table of contents
  - [Overview](#overview)
  - [Setting up access](#gcp-access)
    - [GCP](#gcp-access)
    - [Studio API](#studio-api-access)
  - [Replacing placeholder values](#replacing-placeholder-values)
  - [Executing the Test Plan](#executing-the-test-plan)


## Overview

The Test Plans found in [example-test-plans/router-scale][0] demonstrate how to
run Router performance tests in a way similar to the existing [router-scale][1]
testing tool.

Rather than running as a single Test Plan as we did in [hello world](./hello-world.md),
these tests are run using a _pair_ of Test Plans:
  - The first is a [wrapper][2] that is used to spin up an ephemeral GCP VM
    where the tests will be executed. This Test Plan is decoupled from the actual
    test you are running and simply provides a shared orchestration layer for
    provisioning VMs and setting them up to be able to run an `rtf` Test Plan which
    is rsync'd across for execution on the VM rather than locally on your laptop
    (or directly in CI).
  - The second is the Test Plan that we will actually execute on the VM as the
    test itself. For the purposes of this guide we'll be using the [router-perf][3]
    Test Plan that will spin up a build of the Apollo [Router][4] with mocked
    subgraphs and run a simple performance test against it using [vegeta][5].

As with the "hello, world!" Test Plan you will need a local checkout of the
[runtime-testing-framework][6] repository and you will need to have the `rtf` binary
installed. Please see the details found in the [Getting Started](./index.md) page for
how to get set up if you have not done so already.

Unlike the "hello, world!" example, these Test Plans requires some additional setup and
access to resources which will need to be in place before things will work, so lets sort
that out first.


## GCP Access

You can request access to GCP by using the `/assist` command in Slack to open a
ticket with IT. Once you’re access request has been actioned, navigate to the
`router-performance` project’s IAM page [here][7] and look for your team name
in that list. If your team does not have the `Service Account Token Creator`
role then you will also need to request that using `/assist`.

> An example PR to handle this can be found [here][8],

Once that role has been granted to your team you will need to ensure that you
have the [gcloud CLI][9] installed and configured. This can be done by running
`gcloud init` after completing the installation instructions in the that link
and then following the steps it provides (for a fresh install) or by running
`gcloud auth login` if you already have the CLI installed.

Finally you will also need to make sure the beta gcloud components are installed,
which can be done by running `gcloud components install beta`.

## Studio API Access
In order to run test plans using production data you will need to have elevated
permissions in Studio. This is handled using [SHERRIF][10] and unlike the GCP
access steps above this will need to be completed each time you want to run
tests that use production data.

- Open a ticket with SHERIFF as instructed [here][11] or alternatively just go
  [here][12] directly.
- Select “Access Type: Apollo Admin Access”. You need “Read Only” access to the
  “Prod” environment.
- Once your admin access has been granted you can create a new personal Studio
  API key [here][13]. Note that your elevated permissions are only valid for 12
  hours and that you will need to create a new API key each time you request
  access via SHERIFF.
- Export your new API key as `APOLLO_KEY` using your perferred mechanism for
  managing shell environment variables before running rtf.
  - [direnv][14] is a nice way to do this if you don't have an existing setup
    you are already using.


## Replacing placeholder values

Before either of the Test Plans can be validated and ran the placeholder values
the contain need to be filled in. Attempting to run them before doing this will
deliberately fail validation so they can not be run by accident.

### Wrapper
cd to the root of your checkout of the `runtime-testing-framework` repository
and run the following command to validate the wrapper test plan:
```bash
rtf resolve example-test-plans/router-scale/wrapper/test-plan.yaml \
  --validate
```

Doing so with a clean checkout of the repository should give you the following
(expected) error output:
```
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.37s
     Running `target/debug/rtf resolve example-test-plans/router-scale/wrapper/test-plan.yaml --validate`
 INFO loading and resolving test plan
 INFO checking if templating will work
 INFO applying values
ERROR (scenario.command_section.env_vars.RTF_DIR) invalid templating value invalid value `1`, expected String
```

Open up `example-test-plans/router-scale/wrapper/test-plan.yaml` and set the
`abs_path_rtf_repo` value to the absolute path of your checkout of this repo.
Re-running the above command should now output the resolved test plan in your
terminal.

### Router-perf
Validating the router-perf test plan looks similar but with an additional
argument as we need to manually specify values to use in place of the command
output from the environment setup (this is only required for validation):
```bash
rtf resolve example-test-plans/router-scale/router-perf/test-plan.yaml \
  --validate \
  --values='{ "router_pid": "1", "router_cgroup": "true" }'
```

Again, you should see some expected error output due to placeholder values:
```
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.21s
     Running `target/debug/rtf resolve example-test-plans/router-scale/router-perf/test-plan.yaml --validate '--values={ "router_pid": "1", "router_cgroup": "true" }'`
 INFO loading and resolving test plan
 INFO checking if templating will work
 INFO applying values
 INFO validating test plan
ERROR (subgraphs) the provided string was not a valid graph ref expected a string of the form 'graph_id@variant'
(supergraph.graphql) the provided string was not a valid graph ref expected a string of the form 'graph_id@variant'
(canned_ops.json) the provided string was not a valid graph ref expected a string of the form 'graph_id@variant'
```

> **NOTE**: If you also see errors around no API key being provided then you
> have not exported your API key under the `APOLLO_KEY` environment variable
> (see the [Studio API access](#studio-api-access) section above)

Open up `example-test-plans/router-scale/router-perf/test-plan.yaml` and set
the `graph_ref` value to a valid router-scale graph ref from the [router-scale corpus][15].
Re-running the above command should now output the resolved test plan in your terminal.

With both Test Plans updated you should have a diff that looks something like this:
```diff
diff --git a/example-test-plans/router-scale/router-perf/test-plan.yaml b/example-test-plans/router-scale/router-perf/test-plan.yaml
index 312489c..82e1a64 100644
--- a/example-test-plans/router-scale/router-perf/test-plan.yaml
+++ b/example-test-plans/router-scale/router-perf/test-plan.yaml
@@ -16,7 +16,7 @@ values:
   random_seed: "b5bb5cd521bf12bd5a18fcfc378ae12d25cf9d4d"
   router_version: "v2.3.0"
   # This is a deliberately invalid graph ref so that we will fail validation before running
-  graph_ref: "CHANGE_ME"
+  graph_ref: "muppets@latest"
   rps: "100"
   duration_secs: "20"
   top_n: 20

diff --git a/example-test-plans/router-scale/wrapper/test-plan.yaml b/example-test-plans/router-scale/wrapper/test-plan.yaml
index af201dd..d84f00f 100644
--- a/example-test-plans/router-scale/wrapper/test-plan.yaml
+++ b/example-test-plans/router-scale/wrapper/test-plan.yaml
@@ -8,7 +8,7 @@ values:
   test_plan_dir: "example-test-plans/router-scale/router-perf"
   # This has been deliberately set with an int so it will fail if you try to run without replacing this.
   # This should be the absolute path to your local dir containing RTF
-  abs_path_rtf_repo: 1
+  abs_path_rtf_repo: "/Users/kermit/repos/runtime-testing-framework"
```

## Executing the Test Plan

Now that both Test Plans validate you can use the wrapper Test Plan to execute
a performance test on an ephemeral VM.
```bash
rtf run example-test-plans/router-scale/wrapper/test-plan.yaml
```

Once the `output` directory has been created you can run `tail -f output/vm-log.txt`
in another terminal window to follow the execution and then run
`./example-test-plans/router-scale/wrapper/scripts/gcloud-ssh-wrapper.sh rtf-router-scale`
once the VM is up to get an SSH session started on the VM if desired. (On the VM, the
output of the tests is placed in `~/output/tests/results/` but be aware that the VM is
auto-removed once the tests are complete).

To persist the VM between runs, replace the teardown command in the wrapper test plan
with "echo 1":
```yaml
    teardown:
      command: echo 1
      # command:
      #   name: delete-router-scale-vm.sh
      #   kind: relative_path
      #   path: scripts/delete-router-scale-vm.sh
      #   args:
      #     - "{{ vm_name }}"
```

  [0]: https://github.com/apollographql/runtime-testing-framework/tree/main/example-test-plans/router-scale
  [1]: https://github.com/apollographql/router-scale
  [2]: https://github.com/apollographql/runtime-testing-framework/tree/main/example-test-plans/router-scale/wrapper
  [3]: https://github.com/apollographql/runtime-testing-framework/tree/main/example-test-plans/router-scale/router-perf
  [4]: https://github.com/apollographql/router
  [5]: https://github.com/tsenart/vegeta
  [6]: https://github.com/apollographql/runtime-testing-framework
  [7]: https://console.cloud.google.com/iam-admin/iam?referrer=search&project=router-performance
  [8]: https://github.com/mdg-private/platform-teams/pull/178
  [9]: https://cloud.google.com/sdk/docs/install
  [10]: https://apollographql.atlassian.net/wiki/spaces/SecOps/pages/805404674/SHERIFF
  [11]: https://apollographql.atlassian.net/wiki/spaces/SecOps/pages/805568513
  [12]: https://apollographql.atlassian.net/servicedesk/customer/portal/1/group/3/create/1247
  [13]: https://studio.apollographql.com/user-settings/api-keys
  [14]: https://github.com/direnv/direnv
  [15]: https://github.com/apollographql/router-scale/blob/main/data/router_2.0_launch/corpus.yaml#L7-L46
