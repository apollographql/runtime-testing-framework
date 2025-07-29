# Getting started

In order to build `rtf` from source you will need a local Rust toolchain.
See the Rust [getting started][0] page for details on how to set this up
if you don't have one already.

To build `rtf` from source, start by cloning the repo locally.
```bash
$ git clone git@github.com:apollographql/runtime-testing-framework.git
$ cd runtime-testing-framework
```

You will need a local Rust toolchain. We recommend using [mise][0] to install
the Rust toolchain and other dependencies.
```bash
$ mise trust
```

Once you have Rust set up you can install `rtf` by using cargo:
```bash
$ cargo install --path crates/rtf-cli
```
```

Once that completes you should be able to run `rtf` in your terminal and
see the help output. With that done, you're ready to run some test plans!

## Further examples
Once you have successfully run the examples in this doc, please look at the
examples in the [rtf-morgue][1]. This contains the most up-to-date examples and
how to write test plans for a variety of different test cases.

  [0]: https://mise.jdx.dev/getting-started.html
  [1]: https://github.com/apollographql/rtf-morgue
