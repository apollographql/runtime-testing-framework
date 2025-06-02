//! The various user facing commands exposed through the CLI.
//!
//! We follow the [git CLI design][0] of splitting user facing commands into low level "plumbing"
//! and high level "porcelain" categories.
//!
//! [0]: https://git-scm.com/docs

pub mod plumbing;
pub mod porcelain;
