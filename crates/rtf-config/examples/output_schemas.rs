use rtf_config::formats::RawTestPlanConfig;
use schemars::generate::SchemaSettings;

fn main() {
    let settings = SchemaSettings::draft07();
    let generator = settings.into_generator();
    let schema = generator.into_root_schema_for::<RawTestPlanConfig>();
    let val = schema.to_value();

    let json = serde_json::to_string_pretty(&val).unwrap();

    println!("{json}");
}
