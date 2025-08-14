use rtf_config::providers::file::FileProvider;
use schemars::generate::SchemaSettings;

fn main() {
    let settings = SchemaSettings::draft07();
    let generator = settings.into_generator();
    let schema = generator.into_root_schema_for::<FileProvider>();
    let val = schema.to_value();

    let md = serde_json::to_string_pretty(&val).unwrap();

    println!("{md}");
}
