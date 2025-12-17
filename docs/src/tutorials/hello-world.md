<!-- diataxis-type: tutorial -->

# Hello, world!

## Overview

The [example_test_plans][0] directory in the rtf repository contains a "hello, world!" test plan
that you can edit and run to learn about the concepts and terminology involved with using rtf.

We'll start with templating and running the test plan as it is written. Then, we'll take a quick
look at a couple of simple ways we can make changes to the config files in order to alter its
behaviour.

> For more information on the structure of RTF test plan config files see the
> [Framework / Test Plans][1] page.
>
> For details on how to get started with writing your own test plans from scratch see the
> [Writing test plans][3] section.

The "hello, world!" test plan contains a brief description, a few variables, and references to
scenario and environment config files:

```yaml
{{ #include ../../../example-test-plans/hello-world/test-plan.yaml }}
```

## Pre-Flight Checks

Use the `rtf template` subcommand to pull in the scenario and environment config files referenced by
the test plan in order to see the fully templated file:

```bash
rtf template example-test-plans/hello-world/test-plan.yaml
```

You should see a larger YAML file containing all the information `rtf` needs to be able to run the
test plan. So, let's try running it!

## Running a Test Plan

To run the test plan, use the `run` subcommand. The `-v` option sets the log level to `INFO`:

```bash
rtf run example-test-plans/hello-world/test-plan.yaml -v
```

You should see output similar to this:

```
 INFO loading and resolving test plan
 INFO checking if templating will work
 INFO creating output directory
 INFO executing test plan
 INFO templating environment setup
 INFO checking environment setup
 INFO executing environment setup
env-setup :: hello, world!
 INFO templating scenario and environment teardown commands
 INFO checking scenario and environment teardown commands
 INFO executing scenario
scenario :: hello, darkness my old friend
 INFO executing environment teardown
---
 INFO writing out resolved test plan and variables
 INFO done
```

You should also see that you now have an `output` directory in the directory where you ran `rtf`.
Take a look inside:

```bash
ls output
```

Output:

```
combined-output.txt
providers
resolved-test-plan.yaml
test-plan-variables.json
```

The `providers` directory contains the scripts that were copied from the **file_providers**
specified in our scenario and environment config files. Let's look at the combined output:

```bash
cat output/combined-output.txt
```

Output:

```
env-setup :: hello, world!
scenario :: hello, darkness my old friend
---
```

The scripts used to generate this output are:

#### echo-message.sh

```bash
{{ #include ../../../example-test-plans/hello-world/scripts/echo-message.sh }}
```

#### teardown.sh

```bash
{{ #include ../../../example-test-plans/hello-world/scripts/teardown.sh }}
```

The `combined-output.txt` file is created by the `echo-message.sh` script and amended by the
`teardown.sh` script.

If you run the test plan a second time you will encounter an error: the `output` directory already
exists. This is a safety mechanism to prevent you from accidentally overwriting existing data or
merging the output from multiple runs together. Either remove the existing directory
(`rm -rf output`) or specify a new one using the `--outdir` flag:

```bash
rtf run example-test-plans/hello-world/test-plan.yaml -v --outdir=more_output
```

You should see output similar to before. You can verify both output directories exist:

```bash
ls | grep output
```

Output:

```
more_output
output
```

## Modifying Variables

The test plan defines scalar **values** for templating variables which are then applied to the
scenario and environment config files.

```yaml
{{ #include ../../../example-test-plans/hello-world/test-plan.yaml }}
```

Run the test plan again using the default log level:

```bash
rtf run example-test-plans/hello-world/test-plan.yaml
```

Output:

```
env-setup :: hello, world!
scenario :: hello, darkness my old friend
---
```

Edit the `test-plan.yaml` to change the variable being used for the setup command:

```diff
 variables:
   message: "hello, "
-  setup_subject: "world!"
+  setup_subject: "sailor!"
   scenario_subject: "darkness my old friend"
```

Run the test plan again to see the modified output:

```bash
rm output -rf
rtf run example-test-plans/hello-world/test-plan.yaml
```

Output:

```
env-setup :: hello, sailor!
scenario :: hello, darkness my old friend
---
```

Now, edit the value of the `message` variable to see that it updates the output for both env-setup
and scenario, as they both reference the same shared variable:

```diff
 variables:
-  message: "hello, "
+  message: "say hi to the "
   setup_subject: "world!"
   scenario_subject: "darkness my old friend"
```

```bash
rm output -rf
rtf run example-test-plans/hello-world/test-plan.yaml
```

Output:

```
env-setup :: say hi to the world!
scenario :: say hi to the darkness my old friend
---
```

## Overriding Individual Variables

If you want to override the value of a variable use the `--var` or `--vars` flags to specify
overrides on the command line:

```bash
rtf run example-test-plans/hello-world/test-plan.yaml \
  --var 'message="say hi to the "'
```

Output:

```
env-setup :: say hi to the world!
scenario :: say hi to the darkness my old friend
---
```

We can also provide the flag multiple times to override multiple variables:

```bash
rtf run example-test-plans/hello-world/test-plan.yaml \
  --var 'message="say hi to the "' \
  --var 'setup_subject=sailor!'
```

Output:

```
env-setup :: say hi to the sailor!
scenario :: say hi to the darkness my old friend
---
```

It is also possible to use the `--vars` flag to provide the location of a JSON file containing the
variables you want to override on top of the ones given in the test plan:

```bash
cat example-test-plans/hello-world/variables.json
```

Output:

```json
{
  "message": "say hi to the ",
  "setup_subject": "sailor!"
}
```

```bash
rtf run example-test-plans/hello-world/test-plan.yaml \
  --vars example-test-plans/hello-world/variables.json
```

Output:

```
env-setup :: say hi to the sailor!
scenario :: say hi to the darkness my old friend
---
```

Each of these options is useful in different ways:

- Using `--var` to provide individual variables on the command line allows you to dynamically set
  things using environment variables and other shell commands
- Using `--vars` to provide a JSON file containing multiple variables allows you to define
  variations on a test plan without having to edit or duplicate the test plan. Those variations can
  be stored in version control.

## Matrix Variables

What if we want to define multiple sets of variables and run them _all_ as part of a batch of tests?
For that, `rtf` provides a **matrix** feature that functions in a similar way to matrices in
[GitHub Actions][2].

To convert a **variable** from a single scalar value to an array of values you'd like to use, move
it under the `matrix.dimensions` section of the test plan:

> Remember to also remove it from the `variables` section or your test plan will fail its check!

```diff
 variables:
   message: "hello, "
-  setup_subject: "world!"
   scenario_subject: "darkness my old friend"

+matrix:
+  dimensions:
+    setup_subject: [ "world!", "sailor!" ]
```

Running the test plan with two `setup_subject` variables produces two results:

```bash
rm output -rf
rtf run example-test-plans/hello-world/test-plan.yaml
```

Output:

```
env-setup :: hello, world!
scenario :: hello, darkness my old friend
---
env-setup :: hello, sailor!
scenario :: hello, darkness my old friend
---
```

If we also move the `scenario_subject` into the matrix:

```diff
 variables:
   message: "hello, "
-  setup_subject: "world!"
-  scenario_subject: "darkness my old friend"
+
+matrix:
+  dimensions:
+    setup_subject: [ "world!", "sailor!" ]
+    scenario_subject: [ "darkness my old friend", "is it me you're looking for?" ]
```

We'll get a run for every _combination_ of variables:

```bash
rm output -rf
rtf run example-test-plans/hello-world/test-plan.yaml
```

Output:

```
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

By default, matrix output directories will be named `matrix_variant_$n` with `n` ranging from 1 to
the number of variants present in the matrix. To override this with a custom, more meaningful, name
you can set the `matrix.variant_names` key to generate variant names using a simple templating
syntax:

```yaml
# The following template will generate variants named "one_3", "one_4", "two_3" and "two_4"
matrix:
  variant_names: "${a}_${b}"
  dimensions:
    a: [ "one", "two" ]
    b: [ 3, 4 ]
```

The syntax used for template strings involves placing _matrix dimension names_ inside of `${}` along
with static string content in order to generate a unique name for each variant. The resulting string
is then slugified to remove whitespace and slashes.

[0]: https://github.com/apollographql/runtime-testing-framework/tree/main/example-test-plans
[1]: ../reference/framework/test-plans.md
[2]: https://docs.github.com/en/actions/writing-workflows/choosing-what-your-workflow-does/running-variations-of-jobs-in-a-workflow
[3]: ./test-plans/index.md
