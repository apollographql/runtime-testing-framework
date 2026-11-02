<!-- diataxis-type: reference -->

# Workload Pools

A [Workload Pool][0] is a group of Kubernetes clusters that the Orchestrator runs Test Executions
on. The pool that a run is scheduled in determines which clusters it can use, the rate limits it is
subject to, and whether it can claim a [Dedicated Cluster][1]. This page describes how pools are
configured and how runs are scheduled within them.

## Pool configuration

Each pool defines:

- The clusters in the pool, and how many Test Executions each cluster runs at once
- The per-user [rate limits][2] for the pool
- Whether Scenarios get write access to their namespace by default
- Whether the pool supports Dedicated Clusters

The current configuration for each pool is shown under **Workload pool configuration** in the
[Orchestrator UI][3], and is set in the [Orchestrator configuration file][4].

## Choosing a pool

| Pool         | Used by                                                     | Suitable for                                                               |
| ------------ | ----------------------------------------------------------- | -------------------------------------------------------------------------- |
| Default      | Unregistered Test Plans, and unpinned [Known Test Plans][5] | Functional testing, short-running CI checks, and developing new Test Plans |
| Pinned pools | Known Test Plans pinned to the pool by an Admin             | Performance testing and other workloads sensitive to noisy neighbours      |

Runs of unregistered Test Plans always use the default pool. Only Known Test Plans can be pinned to
another pool, and pinning requires an Admin (see [Orchestrator internal processes][6]).

## Scheduling in a shared pool

Unless a run requires a Dedicated Cluster, each of its Test Executions is scheduled independently
onto the least loaded cluster in the pool that has capacity. All of the services for a single Test
Execution run in the same cluster, but different Test Executions from the same run may run in
different clusters, alongside executions from other runs.

When every cluster in the pool is at capacity, Test Executions wait in the queue until capacity
frees up.

## Dedicated Clusters

A Known Test Plan that requires a Dedicated Cluster claims a whole cluster in its pool for each run.
The pool must support Dedicated Clusters.

- If a cluster in the pool is empty, the run claims it immediately.
- Otherwise, the run reserves a cluster. No new work is scheduled onto a reserved cluster, and the
  run claims it once the executions already running on it have finished.
- All of the run's Test Executions run on the claimed cluster, up to the cluster's concurrency
  limit.
- The cluster is released for other runs once the run completes.

Dedicated Clusters avoid noisy neighbours from other runs and allow the cluster's nodes to be split
between the run's services using [node label weights][7].

## Related pages

- [Known Test Plan configuration][8]
- [Limits, permissions, and concepts][2]
- [Getting set up with the Orchestrator][9]

[0]: ../glossary.md#workload-pool
[1]: ../glossary.md#dedicated-cluster
[2]: limits-permissions-and-concepts.md
[3]: https://api.rtf.apollographql.com/ui
[4]: https://github.com/mdg-private/kanaveral/blob/main/management-cluster/orchestrator/orchestrator-config.yaml
[5]: ../glossary.md#known-test-plan
[6]: internal-processes.md
[7]: known-test-plan-configuration.md#node-label-weights
[8]: known-test-plan-configuration.md
[9]: ../../howto/orchestrator/getting-set-up.md
