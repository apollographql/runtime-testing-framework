# Getting started

In order to build `rtf` from source you will need a local Rust toolchain.
See the Rust [getting started][0] page for details on how to set this up
if you don't have one already.

Once you have Rust set up you can install `rtf` by cloning the git repository
using cargo:
```bash
$ git clone git@github.com:apollographql/runtime-testing-framework.git
$ cd runtime-testing-framework
$ cargo install --path crates/rtf-cli
```

Once that completes you should be able to run `rtf` in your terminal and
see the help output. With that done, you're ready to run some test plans!

  [0]: https://www.rust-lang.org/learn/get-started
