<!-- diataxis-type: reference -->

# Known Test Plan configuration

A [Known Test Plan][0] is a reference to a Test Plan in GitHub, stored by the Orchestrator along
with settings that control who can trigger it and where it runs. Owners and Admins edit these
settings from the **Edit test plan** section of the Known Test Plan's page in the
[Orchestrator UI][1].

## Settings

| Setting                | Admin only | Description                                                                                                       |
| ---------------------- | ---------- | ----------------------------------------------------------------------------------------------------------------- |
| Name                   |            | Unique name for the Known Test Plan.                                                                              |
| Description            |            | Optional description.                                                                                             |
| Org, Repo, and Path    |            | Location of the Test Plan file in GitHub. Each location can only be registered once.                              |
| Triggers               |            | **Unrestricted**, or **Restricted to owners and trigger users**.                                                  |
| Automated triggers     |            | **Allowed**, or **Disabled** to prevent the Known Test Plan from being run from CI.                               |
| Trigger users          |            | Users who can trigger the Known Test Plan when triggers are restricted. One email address per line.               |
| Namespace write access |            | Whether Scenarios can write to Kubernetes resources in their namespace. **Pool default** uses the pool's setting. |
| Node label weights     |            | How to split the nodes of a [Dedicated Cluster][6] between labels. One `label=weight` per line. See below.        |
| Owners                 | Yes        | Users who can edit the Known Test Plan and cancel its runs. One email address per line.                           |
| Pinned workload pool   | Yes        | The [Workload Pool][7] to run in. **Not pinned** uses the default pool.                                           |
| Dedicated cluster      | Yes        | **Required** claims a whole cluster for each run. The pinned pool must support Dedicated Clusters.                |

A user can be an owner or a trigger user, but not both.

## Node label weights

Node label weights split the nodes of a Dedicated Cluster between labels, in proportion to each
label's weight. Services select the nodes for a label using the `rtf.io/required-node-label-key` and
`rtf.io/required-node-label-value` [service labels][2].

Node label weights:

- require the Known Test Plan to be pinned to a pool and to require a Dedicated Cluster
- use Kubernetes label value syntax for each label, and each label must be unique
- must include at least one non-zero weight

For example, to split a cluster's nodes evenly between the services under test and the Scenario:

```text
environment=1
scenario=1
```

## Related pages

- [Registering and managing Known Test Plans][3]
- [Limits, permissions, and concepts][4]
- [Orchestrator internal processes][5]

[0]: ../glossary.md#known-test-plan
[1]: https://api.rtf.apollographql.com/ui
[2]: ../framework/environments.md
[3]: ../../howto/orchestrator/known-test-plans.md
[4]: limits-permissions-and-concepts.md
[5]: internal-processes.md
[6]: ../glossary.md#dedicated-cluster
[7]: ../glossary.md#workload-pool
