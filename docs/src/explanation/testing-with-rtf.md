<!-- diataxis-type: explanation -->

# Where RTF fits in your test suite

Before diving in, it's worth being precise about what RTF actually is — because it's easy to mistake
it for something it isn't.

RTF is a framework for **building and reproducing complex integrated environments**. It handles
spinning up services, resolving credentials, pulling in parameterised test data, and tearing
everything down cleanly afterwards. What it is _not_ is a framework for writing tests. RTF doesn't
provide assertions, test runners, or anything that touches the logic of your test scenarios — that
part is entirely up to you and your existing tooling.

This distinction matters. The value RTF provides is reliability and repeatability at the
_environment_ level: the confidence that every time you run a [Test Plan][3], the services under
test are in the same known-good state, with the same data, in the same configuration. See
[Understanding RTF][0] for more on the design philosophy behind this.

A further benefit of this approach is consistency. When [Environments][3] and Test Plans are defined
in RTF, they can be shared, reviewed, and reproduced by any team — whether that's another
engineering team building on the same services, or Apollo's support team trying to reproduce a
customer issue. Everyone is working from the same definition, in the same way.

Because standing up a full integrated environment has a non-trivial cost, RTF is best suited to
tests that justify that overhead — tests that can only be run meaningfully against a real, running
system. Understanding where that threshold sits in the testing pyramid will help you decide what
belongs in an RTF Test Plan and what doesn't.

| Test type             | ✅ Use RTF                                                                                                           | ❌ Don't use RTF                                                                                                        |
| --------------------- | -------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------- |
| **E2E**               | • Full-stack tests against real running services<br>• Tests requiring real credentials or external dependencies      | • Tests of logic within a single service<br>• Tests that can run meaningfully against a mock or stub                    |
| **Smoke**             | • Post-deployment health checks against a real environment<br>• Confirming environment setup before a full run       | • Exhaustive regression coverage<br>• Checks that need to run in under a second                                         |
| **Performance**       | • Load testing against staging with representative workloads<br>• Comparing throughput across service configurations | • Microbenchmarks of isolated functions<br>• Profiling the internals of a single service                                |
| **Exploratory**       | • Reproducing a bug against a live or staging environment<br>• Testing a hypothesis under a specific configuration   | • Structured regression runs that need a pass/fail record<br>• Investigations that only require reading logs or metrics |
| **Integration tests** | —                                                                                                                    | Tests of internal component boundaries within a single service                                                          |
| **Unit tests**        | —                                                                                                                    | Any test that doesn't require a running service                                                                         |

## The testing pyramid

The testing pyramid is a useful mental model for thinking about how to distribute your automated
tests, [originally described by Martin Fowler][1]. It has three layers:

- **Unit tests** (base) — test the smallest piece of logic in isolation. Fast, numerous, highly
  specific. A failing unit test points directly at the problem.
- **Integration tests** (middle) — test how two or more real components work together. Slower than
  unit tests, but they catch problems that only emerge at the boundaries between components.
- **End-to-end tests** (top) — test the full system from the outside, the way a real user or
  consumer would. The most expensive layer: they're slow, require real infrastructure, and are prone
  to flakiness.

The pyramid shape reflects a practical ratio: many unit tests, fewer integration tests, and a small
number of E2E tests covering your most critical paths. The top of the pyramid is valuable but
costly, so you reserve it for the things that matter most.

## Where RTF lives

RTF operates at the **top of the pyramid**. Every RTF Test Plan requires one or more real, running
services to test against. The services themselves, how they're configured, and how your test
scenarios interact with them are all up to you — RTF is the layer that makes sure they're all in
place, in the right state, before your tests run.

This is intentional. RTF is designed for tests that can't be meaningfully run in isolation — where
the question you're answering is "does this actually work, end to end, in this configuration?"

## Test types RTF is suited for

All of the following test types sit at the top of the pyramid. What differs is their purpose and how
often you'd run them.

### End-to-end suites

The most common use of RTF is running a full E2E suite — a set of [Scenarios][3] that together
verify the critical behaviour of a service across a set of configurations. A good E2E suite covers
the flows your users depend on most, not every possible code path. Because E2E tests are expensive
to run and maintain, selectivity is a virtue.

| Use RTF                                                               | Don't use RTF                                                                   |
| --------------------------------------------------------------------- | ------------------------------------------------------------------------------- |
| Verifying end-to-end behaviour across a full set of running services  | Tests of logic within a single service that don't require a running environment |
| Verifying behaviour across multiple service configurations            | Tests that can run meaningfully against a mock or stub                          |
| Tests requiring real credentials, tokens, or external dependencies    | Tests that cover every possible code path                                       |
| Reproducing customer-facing bugs against a representative environment | Fast regression checks that should run on every commit                          |

### Smoke and diagnostic runs

A smoke run should be just enough to confirm configuration and connectivity of a deployment of your
service is working as expected. You'd typically run smoke tests immediately before and after a
deployment.

In RTF, you can run a smoke suite by writing a dedicated Test Plan that references a subset of your
Scenarios, or by passing specific `--var` values to target a narrower slice of your matrix.

| Use RTF                                                           | Don't use RTF                                    |
| ----------------------------------------------------------------- | ------------------------------------------------ |
| Pre- and Post-deployment health checks against a real environment | Comprehensive regression coverage                |
| Verifying the single most critical path is functional             | Deep scenario coverage or exhaustive matrix runs |
| Confirming environment setup is correct before a full run         | Checks that should run in under a second         |

### Performance runs

RTF can be used for performance and load testing, where the Scenario exercises the service under
representative load rather than verifying correctness alone. Performance tests are typically run on
a different cadence to your functional E2E suite — against a staging environment before a release,
rather than on every commit.

Performance tests are most valuable when they reflect realistic workloads against the actual
services under test — which means they need a real environment and real data. RTF gives you a
reproducible way to stand that environment up consistently.

| Use RTF                                                                  | Don't use RTF                                          |
| ------------------------------------------------------------------------ | ------------------------------------------------------ |
| Load testing against a staging environment with representative workloads | Microbenchmarks of isolated functions or algorithms    |
| Measuring throughput or latency across different service configurations  | Profiling the internals of a single service process    |
| Comparing performance before and after a configuration change            | Performance tests that don't require a running service |

### Exploratory testing

RTF's granular execution flags make it well suited to exploratory testing — running a specific
Scenario against a live service to investigate unexpected behaviour or test a hypothesis. For a
broader treatment of exploratory testing and where it fits alongside automated suites, see
[The Practical Test Pyramid][2].

You can run a single Scenario directly:

```shell
rtf run --scenario test-plan.yaml
```

Or pin `--var` flags to run against a single matrix configuration without executing the full matrix.
This makes RTF useful not just as an automated test runner, but as a tool for debugging and
investigation in live environments.

| Use RTF                                                                     | Don't use RTF                                             |
| --------------------------------------------------------------------------- | --------------------------------------------------------- |
| Reproducing a bug against a live or staging environment                     | Structured regression runs that need a pass/fail record   |
| Testing a hypothesis about service behaviour under a specific configuration | Investigations that only require reading logs or metrics  |
| Manually verifying a fix before promoting to the full suite                 | Debugging issues within a single service's internal logic |

## Everything else

RTF is not the right tool for unit tests or integration tests within a single service. If what
you're testing doesn't require a real running service — a parsing edge case, an internal function, a
validation rule — you should test it with your language's native test tooling. If you don't require
a running service, don't use RTF.

This isn't a limitation of RTF; it's a deliberate boundary. The framework is designed to compose
with your existing tooling, not replace it. A healthy test suite uses RTF alongside unit and
integration tests, not instead of them.

If you're unsure whether something belongs in RTF, ask: does this test require a real service to be
running? If the answer is no, it probably doesn't belong here.

[0]: ./overview.md
[1]: https://martinfowler.com/bliki/TestPyramid.html
[2]: https://martinfowler.com/articles/practical-test-pyramid.html
[3]: ../reference/glossary.md
