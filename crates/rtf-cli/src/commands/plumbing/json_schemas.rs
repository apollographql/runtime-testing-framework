use crate::cli::SchemasConfig;
use rtf_config::formats::{EnvironmentConfig, RawTestPlanConfig, ScenarioConfig};
use schemars::generate::SchemaSettings;

pub fn generate_json_schema(config: SchemasConfig) -> anyhow::Result<()> {
    let settings = SchemaSettings::draft07();
    let generator = settings.into_generator();
    let schema = match config {
        SchemasConfig::TestPlan => generator.into_root_schema_for::<RawTestPlanConfig>(),
        SchemasConfig::Environment => generator.into_root_schema_for::<EnvironmentConfig>(),
        SchemasConfig::Scenario => generator.into_root_schema_for::<ScenarioConfig>(),
    };
    let val = schema.to_value();
    let json = serde_json::to_string_pretty(&val).expect("JSON Value serialization is infallible");

    println!("{json}");

    Ok(())
}
