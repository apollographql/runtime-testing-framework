# The Apollo Runtime Testing Framework

This repository contains the `rtf` command line tool along with associated example Test Plans and
documentation.

The documentation can be viewed in GitHub Pages [here][0].

If you are contributing to this repo, the developer documentation can be found [here][1].

To build `rtf` from source, start by cloning the repo locally.

```bash
$ git clone git@github.com:apollographql/runtime-testing-framework.git
$ cd runtime-testing-framework
```

You will need a local Rust toolchain. We recommend using [mise][2] to install the Rust toolchain and
other dependencies.

```bash
$ mise trust
```

If you are using Rust Rover you can change your rust toolchain in Settings > Rust to point to
`/Users/<USER>/.local/share/mise/shims`

Once you have Rust set up you can install `rtf` by using cargo:

```bash
$ cargo install --path crates/rtf-cli
```

Once that completes you should be able to run `rtf` in your terminal and see the help output. For
more details on getting started with `rtf` please refer to the [docs][0], the source of which are
written using [mdbook][3] in the [docs](./docs) directory.

For examples of what it looks like to write Test Plans using `rtf` please see the [rtf-morgue][4]
directory.

[0]: https://apollographql.github.io/runtime-testing-framework
[1]: https://apollographql.github.io/runtime-testing-framework/developer/explanation/index.html
[2]: https://mise.jdx.dev/getting-started.html
[3]: https://rust-lang.github.io/mdBook/index.html
[4]: https://github.com/apollographql/rtf-morgue
