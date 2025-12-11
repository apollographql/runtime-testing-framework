# The Apollo Runtime Testing Framework

TEST

This repository contains the `rtf` command line tool along with associated example Test Plans and
documentation.

The documentation can be viewed in GitHub Pages [here][0].

To build `rtf` from source, start by cloning the repo locally.

```bash
$ git clone git@github.com:apollographql/runtime-testing-framework.git
$ cd runtime-testing-framework
```

You will need a local Rust toolchain. We recommend using [mise][1] to install the Rust toolchain and
other dependencies.

```bash
$ mise trust
```

Once you have Rust set up you can install `rtf` by using cargo:

```bash
$ cargo install --path crates/rtf-cli
```

Once that completes you should be able to run `rtf` in your terminal and see the help output. For
more details on getting started with `rtf` please refer to the [docs][0], the source of which are
written using [mdbook][2] in the [docs](./docs) directory.

For examples of what it looks like to write Test Plans using `rtf` please see the [rtf-morgue][3]
directory.

[0]: https://apollographql.github.io/runtime-testing-framework
[1]: https://mise.jdx.dev/getting-started.html
[2]: https://rust-lang.github.io/mdBook/index.html
[3]: https://github.com/apollographql/rtf-morgue
