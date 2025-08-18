use clap::CommandFactory;
use clap_complete::aot::{Shell, generate_to};
use clap_markdown::{MarkdownOptions, help_markdown_custom};
use std::{fs, io};

#[path = "src/cli.rs"]
mod cli;

fn main() -> io::Result<()> {
    // Force a rebuild if the named files or directories are modified
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-changed=resources");

    // Write out a markdown version of our help into the root of the crate
    let help = help_markdown_custom::<cli::Args>(&MarkdownOptions::new().show_footer(false));

    fs::write("help.md", help)?;

    // Write out a completion files
    for shell in [Shell::Bash, Shell::Zsh, Shell::Fish] {
        let mut cmd = cli::Args::command();
        let name = cmd.get_name().to_string();

        _ = generate_to(shell, &mut cmd, name, "shell_completions");
    }

    Ok(())
}
