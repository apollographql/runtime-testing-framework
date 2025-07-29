# The Apollo Runtime Testing Framework

This repository contains the `rtf` command line tool along with associated example Test Plans and
documentation.

To build `rtf` from source, start by cloning the repo locally.

```bash
$ git clone git@github.com:apollographql/runtime-testing-framework.git
$ cd runtime-testing-framework
```

You will need a local Rust toolchain. We recommend using [mise][0] to install the Rust toolchain and
other dependencies.

```bash
$ mise trust
```

Once you have Rust set up you can install `rtf` by using cargo:

```bash
$ cargo install --path crates/rtf-cli
```

Once that completes you should be able to run `rtf` in your terminal and see the help output. For
more details on getting started with `rtf` please refer to the [docs](./docs) directory which
contains user facing documentation written using [mdbook][1].

For examples of what it looks like to write Test Plans using `rtf` please see the [rtf-morgue][2]
directory.

[0]: https://mise.jdx.dev/getting-started.html
[1]: https://rust-lang.github.io/mdBook/index.html
[2]: https://github.com/apollographql/rtf-morgue
