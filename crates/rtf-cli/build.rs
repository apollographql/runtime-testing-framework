use clap_markdown::{MarkdownOptions, help_markdown_custom};
use std::{
    fs::{self},
    io,
};

#[path = "src/cli.rs"]
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
