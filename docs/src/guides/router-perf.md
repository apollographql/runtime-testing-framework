# Router Performance Testing

> **Note**: The `router-validation` example contained in this guide is stored in the
> [rtf-morgue][0]. This contains many examples of how to run different styles of tests in RTF. We
> recommend looking through the examples in the morgue to find a test plan that matches your use
> cases(s) once familiar with the steps in this guide.

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

The test plans found in [rtf-morgue/test-plans/router-validation][1] demonstrate how to run Router
performance tests in a way similar to the existing [router-scale][2] testing tool.

These test plans are intended to be run in GCP on a VM, both in order to provide more resources for
running the Router and supporting services, and to ensure that customer data from Studio is not
downloaded to developer laptops.

As with the "hello, world!" test plan you will need to have the `rtf` binary installed. Please see
the details found in the [Getting Started](./index.md) page for how to get set up if you have not
done so already.

Unlike the "hello, world!" example, these test plans requires some additional setup and access to
resources which will need to be in place before things will work, so lets sort that out first.

## GCP Access

At Apollo, GCP access is managed by team. Navigate to the `router-performance` project’s
[IAM page][3] and look for your team name in that list. Verify that your team has the
`Service Account Token Creator` role.

If your team does not have `Service Account Token Creator` role in the `router-performance` project,
use the `/assist` command in Slack to open a ticket with IT. Reference this
[example platform-teams PR][4] if necessary.

Check if you have the gcloud CLI installed:

```bash
which gcloud
```

If you do not have the gcloud CLI installed, follow the [gcloud CLI installation instructions][5].

To authenticate with the gcloud CLI, run:

```bash
gcloud auth login
```

Ensure that beta components are installed:

```bash
gcloud components install beta
```

## Studio API Access

In order to run test plans using production data you will need to have elevated permissions in
Studio. This is handled using [SHERRIF][6]. Unlike the GCP access steps above this will need to be
completed each time you want to run tests that use production data.

- Open a [SHERIFF ticket][7].
  - Summary: "Read-only Apollo admin access for running RTF"
  - Justification: "Elevated permissions are required for running RTF against production data"
  - Access Type: "Apollo Admin Access"
  - Role: "Read Only"
  - Environment(s) "Prod"
- Once your admin access has been granted, create a [new personal Studio API key][8]. Note that your
  elevated permissions are only valid for 12 hours and that you will need to create a new API key
  each time you request access via SHERIFF.
- Export your new API key as `APOLLO_KEY` using your preferred mechanism for managing shell
  environment variables before running rtf.
  - We use [mise][9] in the rtf repo and manage our environment variables in a `.env` file that is
    git ignored. We recommend managing your environment variables in a local `.env` file.

## GitHub Access

In order to run this test plan, you will need a `GITHUB_TOKEN`. This is so that the VM can pull the
RTF binary directly from the GitHub workflow artifacts. This is also required so that GitHub file
providers will run (if being used).

To create one, follow the instructions on creating a [personal access token (classic)][10]. The
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

## Validating the test plans

All test plans in the `rtf-morgue` repo have an accompanying `template-values.json` file that can be
used for locally checking that you have the correct setup in place. cd to the root of your checkout
of the `rtf-morgue` repository and run the following command to validate that everything is set up
correctly:

```bash
rtf template test-plans/router-validation/test-plan.yaml \
  --values template-values.json \
  --check
```

If everything works then you should see the resolved test plan YAML in your terminal, if you are
missing any of the environment variables for GitHub or Studio then you will receive an error
detailing what is missing.

Make sure that you can successfully validate the test plan before continuing.

## Executing the Test Plan

Details on how to provision your GCP VM and execute the test plan on it are provided in the
[README][11] found in the `vm-scripts` sub-directory of `router-validation`.

> **NOTE**: As part of [RR-306][12] this is being overhauled to make working with the GCP VM setup
> easier to onboard with. These docs will be updated to cover that process once it is in place.

[0]: https://github.com/apollographql/rtf-morgue
[1]: https://github.com/apollographql/rtf-morgue/tree/main/test-plans/router-validation
[2]: https://github.com/apollographql/router-scale
[3]: https://console.cloud.google.com/iam-admin/iam?referrer=search&project=router-performance
[4]: https://github.com/mdg-private/platform-teams/pull/178
[5]: https://cloud.google.com/sdk/docs/install
[6]: https://apollographql.atlassian.net/wiki/spaces/SecOps/pages/805404674/SHERIFF
[7]: https://apollographql.atlassian.net/servicedesk/customer/portal/1/group/3/create/1247
[8]: https://studio.apollographql.com/user-settings/api-keys
[9]: https://apollographql.atlassian.net/wiki/spaces/Foundation/pages/1490583600/Mise+standardized+dev+tools+made+simple
[10]: https://docs.github.com/en/authentication/keeping-your-account-and-data-secure/managing-your-personal-access-tokens#creating-a-personal-access-token-classic
[11]: https://github.com/apollographql/rtf-morgue/blob/main/test-plans/router-validation/vm-scripts/README.md
[12]: https://apollographql.atlassian.net/browse/RR-306
