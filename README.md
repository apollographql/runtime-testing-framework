# The Apollo Runtime Testing Framework

This repository contains the `rtf` command line tool along with associated
example Test Plans and documentation.

In order to build `rtf` from source you will need a local Rust toolchain.
See the Rust [getting started][0] page for details on how to set this up
if you don't have one already.

Once you have Rust set up you can install `rtf` by cloning the git repository
and using cargo:
```bash
$ git clone git@github.com:apollographql/runtime-testing-framework.git
$ cd runtime-testing-framework
$ cargo install --path crates/rtf-cli
```

Once that completes you should be able to run `rtf` in your terminal and
see the help output. For more details on getting started with `rtf` please
refer to the [docs](./docs) directory which contains user facing documentation
written using [mdbook][1].

For examples of what it looks like to write Test Plans using `rtf` please
see the [rtf-morgue][2] directory.

  [0]: https://www.rust-lang.org/learn/get-started
  [1]: https://rust-lang.github.io/mdBook/index.html
  [2]: https://github.com/apollographql/rtf-morgue
