# The Apollo Runtime Testing Framework

This repository contains the `rtf` command line tool and `orchestrator` service along with
supporting crates and example Test Plans.

The user facing documentation can be viewed in GitHub Pages [here][0].

### Project status

While this tool may be of interest to users outside of Apollo, it should be noted that RTF is an
Apollo internal tool and not a supported Apollo product.

Please refer to the `LICENSE` file in the root of the repo for further information.

### Contributing to RTF as an Apollo Engineer

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

[0]: https://apollographql.github.io/runtime-testing-framework
[1]: https://apollographql.github.io/runtime-testing-framework/developer/explanation/index.html
[2]: https://mise.jdx.dev/getting-started.html
[3]: https://rust-lang.github.io/mdBook/index.html
