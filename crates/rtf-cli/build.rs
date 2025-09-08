use clap::CommandFactory;
use clap_complete::aot::{Shell, generate_to};
use clap_markdown::{MarkdownOptions, help_markdown_custom};
use rtf_config::formats::{EnvironmentConfig, RawTestPlanConfig, ScenarioConfig};
use schemars::generate::SchemaSettings;
use std::{
    fs::{self, write},
    io,
};

#[path = "src/cli.rs"]
mod cli;

macro_rules! write_schema {
    ($ty:ty, $path:expr) => {
        let settings = SchemaSettings::draft07();
        let generator = settings.into_generator();
        let schema = generator.into_root_schema_for::<$ty>();
        let val = schema.to_value();

        let json = serde_json::to_string_pretty(&val).unwrap();
        write($path, json).unwrap();
    };
}

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

    // Write out JSON schema files for each of the config file formats
    write_schema!(RawTestPlanConfig, "json_schema/test-plan-schema.json");
    write_schema!(EnvironmentConfig, "json_schema/environment-schema.json");
    write_schema!(ScenarioConfig, "json_schema/scenario-schema.json");

    Ok(())
}
