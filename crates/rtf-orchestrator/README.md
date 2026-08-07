# Rep Orchestrator - k8s based execution of RTF Test Plans

This crate contains the `rtf-orchestrator` server that we use to execute RTF test plans inside of
kubernetes clusters managed by REP (the "Runtime Environment Provisioner"). As this is a webserver,
working with this crate is a little different from the others found within this cargo workspace.

## Testing and the local stack

> See the main developer docs section of the RTF docs [here][0] for details that apply for the repo
> as a whole.

In order to support a fast developer feedback loop, we partition the tests for this crate into three
categories:

1. Normal unit tests (run using `cargo test`)

- These are written following the repo wide guidance given [here][1] and align with
  [standard unit testing practices][2].

2. "DB tests" that require a live postgres database (run using `make db-tests`)

- These are written as [normal Rust unit tests][2] in the module containing the logic under tests
  and _must_ be tagged with the `#[cfg_attr(not(feature = "db_tests"), ignore)]` annotation as
  described in [Cargo.toml][3].

3. Integration tests that require the full local stack (run using `make integration-tests`)

- These are written as [Rust integration tests][4] and are only used to validate full execution
  flows of test plans as they need to spin up kubernetes resources in order to run.

### DB tests Vs Unit tests

The purpose of DB tests is to validate that the SQL queries and associated wrapper logic we write
are valid for the database schema we are currently using. Wherever possible we provide private
helper methods for constructing DB query structs directly within unit tests and prefer that approach
for testing logic that doesn't need to directly interact with the database.

DB tests can be executed against the [local stack][5] if it is already spun up, but bringing the
full stack up when we don't need to interact with kubernetes is time consuming and resource
intesive. As a light weight alternative we also have a simple [docker compose setup][6] that can be
started with `make db-up` which is significantly faster to bring up.

```bash
# Example local flow

# Terminal 1
$ make db-up

# Work on DB related code in your editor / IDE of choice

# Terminal 2
$ make db-tests

# When you are done, Ctrl-C in terminal 1 and then
$ make db-down
```

> When run in CI, DB tests are always executed against the full Tilt based stack.

### Integration tests & the local stack

The integration suite is kept as small as possible given the need for each test case to spin up
resources inside of locally running kubernetes clusters. That said, the local stack is a useful
development tool in its own right both as a debugging tool and for manually verifying changes to the
behaviour of the service.

> We recommend using [colima][7] for running docker on your Apollo MacBook. The default resources
> colima runs with are insufficient for the local stack so you will need bump them up by running the
> following:
>
> ```bash
> $ brew install colima
> $ colima stop
> $ colima start --cpu 4 --memory 8
> ```

```bash
# Example local flow

# Terminal 1
$ make cluster-setup
$ make cluster-up
# hit spacebar to open the Tilt web UI

# Terminal 2
# Running automated tests
$ make integration-tests # only run the integration tests
$ make test-all          # run all unit, DB & integration tests
# Manual testing (server running on localhost:8035)
$ k9s --context kind-rtf-mgmt # use k9s to view activity in the local clusters

# When you are done, Ctrl-C in terminal 1 and then
$ make cluster-down
$ make cluster-teardown
```

### K9s cheatsheet

[This][8] cheatsheet is helpful for learning the basics of using [k9s][9]. In addition to that, the
following commands are useful for specific tasks relating to our local stack:

```bash
:context  # switch between the management and workload clusters
:workflow # view the argo workflows that deploy environments (when in the management cluster)
:job      # view the k8s jobs that run scenarios (when in the workload cluster)
```

> `ctrl-d` when highlighting a resource in k9s will prompt for you to delete it. To manually cleanup
> your local cluster state it is best to do this against namespaces (for the workload cluster) or
> workflows (for the management cluster).

## Architecture

[This confluence page][10] outlines the overall design for the orchestrator service, with
[this figma][11] showing the different moving parts within the service. At a high level, the
orchestrator is currently comprised of three long lived tokio tasks that communicate via MPSC
channels:

1. An axum server that provides the user facing API for submitting test plans for execution and
   querying the status / results of those test plans.
2. A "resolver" task that is responsible for running the RTF related logic that requires API keys
   for interaction with external services such as Studio and GitHub.
3. An event loop task that processes each `test execution` via a series of handlers that each
   interact with one of two kubernetes clusters (management & workload).

Hopefully the existence of the event loop task and use of MPSC channels makes it clear that the
overall design of the server is _event_ driven rather than _request_ driven. This fits naturally
with Kubernetes itself and also allows us to decouple the behaviour of each part of the system,
making testing simpler. As a trade off, we need to make sure that we are thinking about this as a
mini distributed system and being mindful of how that affects our ability to reason about and debug
its behaviour.

[0]: https://scaling-dollop-ywevlle.pages.github.io/developer/explanation/index.html
[1]: https://scaling-dollop-ywevlle.pages.github.io/developer/reference/testing/index.html
[2]: https://doc.rust-lang.org/book/ch11-03-test-organization.html#unit-tests
[3]: ./Cargo.toml
[4]: https://doc.rust-lang.org/book/ch11-03-test-organization.html#integration-tests
[5]: ./local-stack/README.md
[6]: ./local-stack/db/docker-compose.yaml
[7]: https://github.com/abiosoft/colima?tab=readme-ov-file#getting-started
[8]: https://www.hackingnote.com/en/cheatsheets/k9s/
[9]: https://k9scli.io/
[10]: https://apollographql.atlassian.net/wiki/spaces/RUNTIMEREADINESS/pages/2131099659/RTF+Test+Orchestrator
[11]: https://www.figma.com/board/2tHH7PhKfY4lh1k2Dys3rw/R-TF%7CEP-?node-id=114-111&p=f&t=V3yVDTfzk50TKOXo-0
