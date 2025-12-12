<!-- diataxis-type: tutorial -->

# Getting started

To build `rtf` from source, start by cloning the repo locally:

```bash
git clone git@github.com:apollographql/runtime-testing-framework.git
cd runtime-testing-framework
```

You will need a local Rust toolchain. We recommend using [mise][0] to install the Rust toolchain and
other dependencies. After installing `mise`, trust the mise config file:

```bash
mise trust
```

Install `rtf` with cargo:

```bash
cargo install --path crates/rtf-cli
```

Verify installation:

```bash
rtf
```

This should print the rtf help output to your terminal. With that done, you're ready to run some
test plans!

## Example test plans

This repo contains a "Hello, world!" test plan to introduce you to rtf concepts. Start by following
the ["Hello, world!"][2] guide.

The [rtf-morgue][1] contains the most up-to-date examples of how to write test plans for a variety
of different test cases. It also contains guides for running those test plans.

[0]: https://mise.jdx.dev/getting-started.html
[1]: https://github.com/apollographql/rtf-morgue
[2]: ./hello-world.md
