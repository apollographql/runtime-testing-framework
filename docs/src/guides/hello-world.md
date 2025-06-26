# Hello, world!

### Table of contents
  - [Overview](#overview)
  - [Pre-Flight Checks](#pre-flight-checks)
  - [Running a Test Plan](#running-a-test-plan)
  - [Modifying Values](#modifying-values)
  - [Matrix Values](#matrix-values)

## Overview

The [example_test_plans][0] directory in the rtf repository contains several
tests plans that you can try running and editing to learn about the concepts
and terminology involved with using the tool. We'll start with the "hello,
world!" test plan (that you can probably guess the behaviour of) which can
be found at [example-test-plans/hello-world/test-plan.yaml][1].

We're going to start with simply resolving and running the Test Plan as it
is written, before taking a quick look at a couple of simple ways we can
make changes to the config files in order to alter its behaviour.

> For more information on the structure of RTF Test Plan config files see
> the [Test Plans](../test-plans/index.md) page.

If we look at the Test Plan itself you should find that as you might expect
for a "hello, world!" example there isn't a lot in there:

```yaml
{{ #include ../../../example-test-plans/hello-world/test-plan.yaml }}
```

## Pre-Flight Checks

We can use the `rtf resolve` subcommand to pull in the scenario and environment
config files referenced by the test plan in order to see the full thing. Try
running the following from the root of the repository:
```
rtf resolve example-test-plans/hello-world/test-plan.yaml
```

You should see a larger YAML file containing all of the information `rtf` needs
to be able to run the Test Plan. So, lets try running it!


## Running a Test Plan

Running a Test Plan is as simple as replacing the `resolve` subcommand in the
example above with `run`. If you now run that from the root of the repository
you should see the following:
```
rtf run example-test-plans/hello-world/test-plan.yaml

 INFO loading and resolving test plan
 INFO checking if templating will work
 INFO creating output directory
 INFO executing test plan
 INFO resolving environment setup
 INFO validating environment setup
 INFO executing environment setup
>>> Hello from env-setup!
 INFO resolving scenario and environment teardown commands
 INFO validating scenario and environment teardown commands
 INFO executing scenario
>>> Hello from scenario!
 INFO executing environment teardown
>>> Hello from env-teardown!
 INFO done
```

You should also see that you now have an `output` directory in the directory
you ran `rtf` from. Lets take a look inside:
```bash
$ ls output
combined-output.txt  echo-message.sh  teardown.sh

$ cat output/combined-output.txt
env-setup :: hello, world!
scenario :: hello, darkness my old friend
---
```

There's nothing special about the `combined-output.txt` file here: it is just
being created by the test script we are using in the test plan. The `echo-message.sh`
and `teardown.sh` scripts have come from the **FileProviders** specified in our scenario
and environment config files:


#### echo-message.sh
```bash
{{ #include ../../../example-test-plans/hello-world/scripts/echo-message.sh }}
```

#### teardown.sh
```bash
{{ #include ../../../example-test-plans/hello-world/scripts/teardown.sh }}
```

If you try running the Test Plan a second time you will see that you get an
error warning you that the `output` directory already exists. This is a safety
mechanism in place to prevent you accidentally overwriting existing data or
merging the output from multiple runs together. We can either remove the
directory or specify a new one using the `--outdir` flag:
```
$ rtf run 'example-test-plans/hello-world/test-plan.yaml' --outdir=more_output

 INFO loading and resolving test plan
 INFO checking if templating will work
 INFO creating output directory
 INFO executing test plan
 INFO resolving environment setup
 INFO validating environment setup
 INFO executing environment setup
>>> Hello from env-setup!
 INFO resolving scenario and environment teardown commands
 INFO validating scenario and environment teardown commands
 INFO executing scenario
>>> Hello from scenario!
 INFO executing environment teardown
>>> Hello from env-teardown!
 INFO done

$ ls | grep output

more_output
output
```

## Modifying Values

If we look back at the Test Plan file itself we can see that we are defining
some scalar **values** which are then being applied to the scenario and
environment config files.

```yaml
{{ #include ../../../example-test-plans/hello-world/test-plan.yaml }}
```

If we run the Test Plan again with quieter logging we can see just the output
from the scripts being executed:
```bash
$ RUST_LOG=warn rtf run 'example-test-plans/hello-world/test-plan.yaml'

env-setup :: hello, world!
scenario :: hello, darkness my old friend
---
```

Lets edit the `test-plan.yaml` to change the value being used for the setup command:
```diff
 values:
   message: "hello, "
-  setup_subject: "world!"
+  setup_subject: "sailor!"
   scenario_subject: "darkness my old friend"
```

If we run the Test Plan again we should see that we have new output:
```
$ rm output -rf
$ RUST_LOG=warn rtf run 'example-test-plans/hello-world/test-plan.yaml'

env-setup :: hello, sailor!
scenario :: hello, darkness my old friend
---
```

And if we instead edit the `message` value you should see that it updates
the output for both the scenario and the environment setup (as they both
reference the same shared value):
```diff
 values:
-  message: "hello, "
+  message: "say hi to the "
   setup_subject: "world!"
   scenario_subject: "darkness my old friend"
```

```
$ rm output -rf
$ RUST_LOG=warn rtf run 'example-test-plans/hello-world/test-plan.yaml'

env-setup :: say hi to the world!
scenario :: say hi to the darkness my old friend
---
```

## Matrix Values

But what if we want to use multiple messages without having to edit the Test
Plan each time we run it? `rtf` provides a **matrix** feature that functions in
a similar way to matrices in [GitHub Actions][2].

All we need to do to convert a **value** from a single value to a matrix is
move it under the `matrix` section of our Test Plan and provide the array of
values we'd like to use:

> Remember to also remove it from the `values` section or your Test Plan will
> fail to validate!

```diff
 values:
   message: "hello, "
-  setup_subject: "world!"
   scenario_subject: "darkness my old friend"

+matrix:
+  setup_subject: [ "world!", "sailor!" ]
```

If we run the test plan now we should see something a little different from before:
```
$ rm output -rf
$ RUST_LOG=warn rtf run 'example-test-plans/hello-world/test-plan.yaml'

env-setup :: hello, world!
scenario :: hello, darkness my old friend
---
env-setup :: hello, sailor!
scenario :: hello, darkness my old friend
---
```

Now we're getting a run of the full test plan for each of the values we have
provided in the **matrix**. If we also move the `scenario_subject` into the
matrix:
```diff
 values:
   message: "hello, "
-  setup_subject: "world!"
-  scenario_subject: "darkness my old friend"
+
+matrix:
+  setup_subject: [ "world!", "sailor!" ]
+  scenario_subject: [ "darkness my old friend", "is it me you're looking for?" ]
```

Then we'll get a run for every _combination_ of values:
```
$ rm output -rf
$ RUST_LOG=warn rtf run 'example-test-plans/hello-world/test-plan.yaml'

env-setup :: hello, world!
scenario :: hello, darkness my old friend
---
env-setup :: hello, sailor!
scenario :: hello, darkness my old friend
---
env-setup :: hello, world!
scenario :: hello, is it me you're looking for?
---
env-setup :: hello, sailor!
scenario :: hello, is it me you're looking for?
---
```

## Next Steps

> **TODO** Link to the page for running the router-scale test plan and details on
> config structure once they are written.

  [0]: https://github.com/apollographql/runtime-testing-framework/tree/main/example-test-plans
  [1]: https://github.com/apollographql/runtime-testing-framework/tree/main/example-test-plans/hello-world/test-plan.yaml
  [2]: https://docs.github.com/en/actions/writing-workflows/choosing-what-your-workflow-does/running-variations-of-jobs-in-a-workflow
