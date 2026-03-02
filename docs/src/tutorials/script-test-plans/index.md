<!-- diataxis-type: tutorial -->

# Writing script-based test plans

This section guides you through writing a script-based [Test Plan][0] from a blank file. Script
environments and scenarios run shell commands directly on the host instead of using `docker compose`
and containers.

## When to use script-based test plans

The [docker compose tutorial][1] is the recommended starting point. Script-based test plans are for
cases where docker isn't available or isn't suitable — for example, when testing infrastructure that
already manages its own lifecycle, or in environments where docker is not permitted.

> **Prerequisites**
>
> - Completed the ["Writing test plans"][1] tutorial
> - RTF CLI installed and available in your terminal

---

**Next:** [Writing a test plan][2]

[0]: ../../reference/glossary.md
[1]: ../test-plans/index.md
[2]: writing-a-test-plan.md
