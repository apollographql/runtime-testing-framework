<!-- diataxis-type: reference -->

# Orchestrator internal processes

> **Note**: This page describes Apollo internal processes. Concrete details are maintained in
> internal Confluence documentation linked below.

Some Orchestrator functionality depends on setup or configuration that only the Runtime Readiness
team or an Orchestrator admin can provide. This page summarizes each of the internal processes
involved and links to the documentation that describes how to follow them.

## Access

| Process                       | Summary                                                                                      |
| ----------------------------- | -------------------------------------------------------------------------------------------- |
| [Orchestrator access][0]      | Granting a user access to the Orchestrator API and the [Orchestrator UI][1].                 |
| [CI access][2]                | Setting up a repository so that its GitHub Actions workflows can trigger Known Test Plans.   |
| [Private image publishing][3] | Allowing a repository to publish Docker images to Apollo's internal registry for Test Plans. |

## Orchestrator configuration

Rate limits, Workload Pools, and cluster configuration are set in the
[Orchestrator configuration file][4]. To request a change, start a conversation in
[#proj-runtime-testing-framework][5]. If the change is going to go forward, raise a Jira ticket in
the `RR` backlog to track it.

This process covers changes such as:

| Change                            | Summary                                                                               |
| --------------------------------- | ------------------------------------------------------------------------------------- |
| GitHub repository access          | Allowing the Orchestrator to read Test Plans from a GitHub repository.                |
| Rate limit changes                | Changing the per-user [rate limits][6] for a Workload Pool.                           |
| Workload cluster capacity         | Provisioning or scaling clusters and Workload Pools.                                  |
| Additional Kubernetes permissions | Granting Scenarios Kubernetes permissions beyond write access to their own namespace. |
| Toolbox image changes             | Adding tools to the image used to run Test Executions.                                |

## Bugs and feature requests

Report bugs and request new Orchestrator functionality in [#proj-runtime-testing-framework][5].

## Related pages

- [Limits, permissions, and concepts][6]
- [Registering and managing Known Test Plans][7]

[0]: https://apollographql.atlassian.net/wiki/spaces/RUNTIMEREADINESS/pages/2146893825/GCP+Access+-+Release+Tooling
[1]: https://api.rtf.apollographql.com/ui
[2]: https://apollographql.atlassian.net/wiki/spaces/RUNTIMEREADINESS/pages/2139947048/Onboarding+To+Our+Tooling
[3]: https://apollographql.atlassian.net/wiki/spaces/RUNTIMEREADINESS/pages/2139553811/Build+Publish
[4]: https://github.com/mdg-private/kanaveral/blob/main/management-cluster/orchestrator/orchestrator-config.yaml
[5]: https://apollograph.slack.com/archives/C08UBLUD19C
[6]: limits-permissions-and-concepts.md
[7]: ../../howto/orchestrator/known-test-plans.md
