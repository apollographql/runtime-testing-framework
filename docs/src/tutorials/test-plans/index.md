<!-- diataxis-type: tutorial -->

# Writing test plans

This section will guide you through writing a test plan for RTF from a blank file. The test plan
will be a simple example that introduces you to the concepts of writing RTF test plans using a
`docker compose` environment and `docker` scenario. For examples of test plans that can be used for
specific testing use cases, please refer to the [morgue][0].

> **Prerequisites**
>
> - Recommended: completed the ["Hello, World!"][1] guide — ensures you're familiar with the RTF CLI
>   and have it installed
> - `docker` and `docker compose` available locally with your docker daemon running

```bash
docker info
docker compose --help
```

> **Note** This tutorial uses `docker` and `docker compose` as execution backends. RTF parameterizes
> these tools from your config — this tutorial does not teach Docker or Docker Compose themselves.
> Refer to the [Docker documentation][2] and the [Docker Compose documentation][3] for details on
> those tools.

---

**Next:** [Writing a test plan][4]

[0]: https://github.com/apollographql/rtf-morgue
[1]: ../hello-world.md
[2]: https://docs.docker.com/
[3]: https://docs.docker.com/compose/
[4]: writing-a-test-plan.md
