use clap_markdown::{MarkdownOptions, help_markdown_custom};
use std::{
    fs::{self},
    io,
};

// The #[path = ...] macro here is required for us to pull in the cli::Args struct and generate the
// help docs. Annoyingly, the way that works is as a distinct module so we end up with a "dead code"
// warning for methods in that file despite them being used in the CLI itself.
// If we instead try to directly import the module from rtf-cli then we end up depending on the
// crate that this build.rs is a pre-req for and rustc gets (understandably) sad.
#[path = "src/cli.rs"]
#[allow(dead_code)]
mod cli;

fn main() -> io::Result<()> {
    // Force a rebuild if the named files or directories are modified
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-changed=resources");

    // Write out a markdown version of our help into the root of the crate
    let help = help_markdown_custom::<cli::Args>(&MarkdownOptions::new().show_footer(false));

    fs::write("help.md", help)?;

    Ok(())
}
