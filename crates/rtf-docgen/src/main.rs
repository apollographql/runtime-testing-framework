//! Generator for the file provider reference page in the docsite using from JSON schema for
//! the [FileProvider] enum.
use anyhow::{Result, anyhow, bail};
use rtf_config::providers::file::FileProvider;
use schemars::generate::SchemaSettings;
use serde_json::{Map, Value};
use std::io::{self, stdout};

const PAGE_TITLE: &str = "File Providers";
const DEFS_PREFIX: &str = "#/$defs/";
/// Prefix of the schema names given to `Field<T>`, which are documented as plain values.
const TEMPLATABLE_PREFIX: &str = "Templatable ";

fn main() -> anyhow::Result<()> {
    let schema = SchemaSettings::default()
        .into_generator()
        .into_root_schema_for::<FileProvider>()
        .to_value();

    let mut writer = DocWriter::new(&schema, stdout())?;

    writer.write_page()
}

struct DocWriter<'a, W>
where
    W: io::Write,
{
    defs: &'a Map<String, Value>,
    providers: Vec<Provider<'a>>,
    visiting: Vec<&'a str>,
    out: W,
}

impl<'a, W> DocWriter<'a, W>
where
    W: io::Write,
{
    fn new(root: &'a Value, out: W) -> Result<Self> {
        let defs = root
            .get("$defs")
            .and_then(Value::as_object)
            .ok_or_else(|| anyhow!("the FileProvider schema should contain $defs"))?;
        let variants = root
            .get("oneOf")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow!("the FileProvider schema should be a oneOf of its variants"))?;

        let providers = variants
            .iter()
            .map(|variant| Provider::try_new(variant, defs))
            .collect::<Result<_>>()?;

        Ok(Self {
            defs,
            providers,
            visiting: Vec::new(),
            out,
        })
    }

    fn write_page(&mut self) -> Result<()> {
        writeln!(self.out, "<!-- diataxis-type: reference -->\n")?;
        writeln!(self.out, "# {PAGE_TITLE}\n")?;
        writeln!(self.out, "Available file providers:\n")?;

        for provider in self.providers.iter() {
            writeln!(self.out, "{}", toc_link(provider.title))?;
        }

        writeln!(self.out)?;

        for p in self.providers.clone() {
            self.write_provider_section(p)?;
        }

        Ok(())
    }

    fn write_provider_section(
        &mut self,
        Provider {
            kind,
            def_name,
            title,
            def,
        }: Provider<'a>,
    ) -> anyhow::Result<()> {
        writeln!(self.out, "## {title}\n")?;
        writeln!(self.out, "`kind: {kind}`\n")?;
        if let Some(description) = description(def) {
            writeln!(self.out, "{description}\n")?;
        }

        self.visiting.push(def_name);
        self.document_content(def, "##")?;
        self.visiting.pop();

        Ok(())
    }

    fn document_content(&mut self, schema: &'a Value, header: &str) -> Result<()> {
        if let Some(properties) = schema.get("properties").and_then(Value::as_object)
            && !properties.is_empty()
        {
            let required = required_fields(schema);
            if properties.values().any(|prop| description(prop).is_some()) {
                writeln!(self.out, "<details>")?;
                writeln!(self.out, "<summary>Fields</summary>\n")?;
                self.document_fields(properties, &required, &format!("{header}#"))?;
                writeln!(self.out, "</details>\n")?;
            } else {
                self.document_fields(properties, &required, header)?;
            }
        }

        // Untagged enums (anyOf) are an implementation detail that we don't document
        if let Some(variants) = schema.get("oneOf").and_then(Value::as_array) {
            self.document_variants(variants, header)?;
        }

        Ok(())
    }

    fn document_fields(
        &mut self,
        properties: &'a Map<String, Value>,
        required: &[&str],
        header: &str,
    ) -> Result<()> {
        let mut fields: Vec<_> = properties
            .iter()
            .map(|(name, prop)| (required.contains(&name.as_str()), name, prop))
            .collect();
        fields.sort_by_key(|&(is_required, name, _)| (!is_required, name));

        for (is_required, name, prop) in fields.into_iter() {
            if let Some(docs) = description(prop) {
                let tag = if is_required { " (required)" } else { "" };
                writeln!(self.out, "{header} `{name}`{tag}\n")?;
                writeln!(self.out, "{docs}\n")?;
            }

            self.document_field_type(prop, header)?;
        }

        Ok(())
    }

    /// Documents the type behind a field, if it refers to a definition worth documenting.
    fn document_field_type(&mut self, schema: &'a Value, header: &str) -> Result<()> {
        let reference = match referenced_def(schema) {
            Some(reference) => reference,
            None => return Ok(()),
        };
        let (def_name, def) = lookup_def(self.defs, reference)?;

        if def_name.starts_with(TEMPLATABLE_PREFIX) || self.visiting.contains(&def_name) {
            return Ok(());
        }

        if let Some(provider) = self.providers.iter().find(|p| p.def_name == def_name) {
            writeln!(self.out, "{}", toc_link(provider.title))?;
            return Ok(());
        }

        self.visiting.push(def_name);
        self.document_content(def, header)?;
        self.visiting.pop();

        Ok(())
    }

    fn document_variants(&mut self, variants: &'a [Value], header: &str) -> Result<()> {
        writeln!(self.out, "<details>")?;
        writeln!(self.out, "<summary>Variants</summary>\n")?;

        let mut providers = Vec::new();
        let mut struct_variants = Vec::new();

        for variant in variants.iter() {
            if let Some((_, reference)) = tagged_variant(variant) {
                let (def_name, _) = lookup_def(self.defs, reference)?;
                let provider = self
                    .providers
                    .iter()
                    .find(|p| p.def_name == def_name)
                    .ok_or_else(|| anyhow!("{def_name} is not a FileProvider variant"))?;
                providers.push(provider.title);
            } else if let Some(value) = variant.get("const").and_then(Value::as_str) {
                write!(self.out, "- `{value}`")?;
                if let Some(docs) = description(variant) {
                    write!(self.out, ": {}", docs.replace('\n', "  "))?;
                }
                writeln!(self.out)?;
            } else if let Some((name, inner)) = single_key_variant(variant) {
                struct_variants.push((variant, name, inner));
            } else {
                bail!("unsupported enum variant schema: {variant}");
            }
        }

        if providers.len() == self.providers.len() {
            writeln!(self.out, "- Any [file provider]({})", anchor(PAGE_TITLE))?;
        } else {
            for title in providers.iter() {
                writeln!(self.out, "{}", toc_link(title))?;
            }
        }
        writeln!(self.out, "\n")?;

        for (variant, name, inner) in struct_variants.iter() {
            writeln!(self.out, "{header}# `{name}`\n")?;
            if let Some(docs) = description(variant) {
                writeln!(self.out, "{docs}\n")?;
            }
            self.document_field_type(inner, &format!("{header}#"))?;
        }

        writeln!(self.out, "</details>\n")?;

        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
struct Provider<'a> {
    kind: &'a str,
    def_name: &'a str,
    title: &'a str,
    def: &'a Value,
}

impl<'a> Provider<'a> {
    fn try_new(variant: &'a Value, defs: &'a Map<String, Value>) -> anyhow::Result<Self> {
        let (kind, reference) = tagged_variant(variant)
            .ok_or_else(|| anyhow!("FileProvider variants must wrap a struct: {variant}"))?;
        let (def_name, def) = lookup_def(defs, reference)?;
        let title = def
            .get("title")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("{def_name} docs must start with a `# Title` line"))?;

        Ok(Self {
            kind,
            def_name,
            title,
            def,
        })
    }
}

fn lookup_def<'a>(defs: &'a Map<String, Value>, reference: &str) -> Result<(&'a str, &'a Value)> {
    let name = reference.replace("%20", " ");

    defs.get_key_value(&name)
        .map(|(name, def)| (name.as_str(), def))
        .ok_or_else(|| anyhow!("missing schema definition for {name}"))
}

fn description(schema: &Value) -> Option<&str> {
    schema.get("description").and_then(Value::as_str)
}

fn required_fields(schema: &Value) -> Vec<&str> {
    schema
        .get("required")
        .and_then(Value::as_array)
        .map(|names| names.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default()
}

fn tagged_variant(variant: &Value) -> Option<(&str, &str)> {
    let kind = variant.pointer("/properties/kind/const")?.as_str()?;

    Some((kind, referenced_def(variant)?))
}

fn single_key_variant(variant: &Value) -> Option<(&str, &Value)> {
    let properties = variant.get("properties")?.as_object()?;
    if properties.len() != 1 {
        return None;
    }

    properties.iter().next().map(|(k, v)| (k.as_str(), v))
}

/// The name of the definition a field refers to, looking through optional values and lists.
fn referenced_def(schema: &Value) -> Option<&str> {
    if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
        return reference.strip_prefix(DEFS_PREFIX);
    }

    if let Some([inner]) = schema
        .get("allOf")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
    {
        return referenced_def(inner);
    }

    if let Some(options) = schema.get("anyOf").and_then(Value::as_array)
        && let [a, b] = options.as_slice()
    {
        let is_null = |v: &Value| v.get("type").and_then(Value::as_str) == Some("null");
        return match (is_null(a), is_null(b)) {
            (false, true) => referenced_def(a),
            (true, false) => referenced_def(b),
            _ => None,
        };
    }

    schema.get("items").and_then(referenced_def)
}

fn toc_link(title: &str) -> String {
    format!("- [{title}]({})", anchor(title))
}

/// The anchor mdBook generates for a heading.
fn anchor(title: &str) -> String {
    let id: String = title
        .chars()
        .filter_map(|c| match c {
            ' ' => Some('-'),
            c if c.is_alphanumeric() || c == '-' || c == '_' => Some(c.to_ascii_lowercase()),
            _ => None,
        })
        .collect();

    format!("#{id}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn provider_variant(kind: &str, def_name: &str) -> Value {
        json!({
            "type": "object",
            "$ref": format!("{DEFS_PREFIX}{def_name}"),
            "properties": { "kind": { "type": "string", "const": kind } },
            "required": ["kind"],
        })
    }

    fn render_providers(defs: Value) -> String {
        let variants: Vec<_> = defs
            .as_object()
            .unwrap()
            .iter()
            .filter(|(_, def)| def.get("title").is_some())
            .map(|(name, _)| provider_variant(&name.to_lowercase(), name))
            .collect();

        let schema = json!({ "oneOf": variants, "$defs": defs });
        let mut writer = DocWriter::new(&schema, Vec::new()).unwrap();
        writer.write_page().unwrap();

        String::from_utf8(writer.out).unwrap()
    }

    #[test]
    fn render_writes_toc_kind_and_description() {
        let out = render_providers(json!({
            "Remote": {
                "title": "Remote file",
                "description": "A file somewhere else.",
                "type": "object",
                "properties": {},
            },
        }));

        assert!(out.contains("- [Remote file](#remote-file)\n"), "{out}");
        assert!(
            out.contains("## Remote file\n\n`kind: remote`\n\nA file somewhere else.\n"),
            "{out}"
        );
    }

    #[test]
    fn render_tags_required_fields() {
        let out = render_providers(json!({
            "Remote": {
                "title": "Remote file",
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "Where to find it." },
                    "retries": { "type": "integer", "description": "How often to retry." },
                },
                "required": ["url"],
            },
        }));

        assert!(out.contains("### `url` (required)\n"), "{out}");
        assert!(out.contains("### `retries`\n"), "{out}");
    }

    #[test]
    fn render_lists_required_fields_before_optional_fields_alphabetically() {
        let out = render_providers(json!({
            "Remote": {
                "title": "Remote file",
                "type": "object",
                "properties": {
                    "a_opt": { "type": "string", "description": "Optional a." },
                    "b_req": { "type": "string", "description": "Required b." },
                    "c_opt": { "type": "string", "description": "Optional c." },
                    "d_req": { "type": "string", "description": "Required d." },
                },
                "required": ["d_req", "b_req"],
            },
        }));

        let fields = [
            "`b_req` (required)",
            "`d_req` (required)",
            "`a_opt`",
            "`c_opt`",
        ]
        .map(|field| out.find(field).unwrap_or_else(|| panic!("{field} missing")));

        assert!(fields.is_sorted(), "{out}");
    }

    #[test]
    fn render_links_to_nested_file_providers() {
        let out = render_providers(json!({
            "Remote": {
                "title": "Remote file",
                "type": "object",
                "properties": {},
            },
            "Wrapper": {
                "title": "Wrapper",
                "type": "object",
                "properties": {
                    "inner": {
                        "description": "The wrapped provider.",
                        "allOf": [{ "$ref": "#/$defs/Remote" }],
                    },
                },
            },
        }));

        assert!(
            out.contains("### `inner`\n\nThe wrapped provider.\n\n- [Remote file](#remote-file)\n"),
            "{out}"
        );
    }

    #[test]
    fn render_documents_optional_enum_variants() {
        let out = render_providers(json!({
            "Remote": {
                "title": "Remote file",
                "type": "object",
                "properties": {
                    "format": {
                        "description": "The format to use.",
                        "anyOf": [{ "$ref": "#/$defs/Format" }, { "type": "null" }],
                    },
                },
            },
            "Format": {
                "oneOf": [
                    { "type": "string", "const": "json", "description": "JSON\noutput." },
                    {
                        "type": "object",
                        "description": "A custom format.",
                        "properties": { "custom": { "type": "string" } },
                        "required": ["custom"],
                    },
                ],
            },
        }));

        assert!(out.contains("- `json`: JSON  output.\n"), "{out}");
        assert!(out.contains("#### `custom`\n\nA custom format.\n"), "{out}");
    }

    #[test]
    fn render_skips_templatable_fields() {
        let out = render_providers(json!({
            "Remote": {
                "title": "Remote file",
                "type": "object",
                "properties": {
                    "url": {
                        "description": "Where to find it.",
                        "allOf": [{ "$ref": "#/$defs/Templatable%20string" }],
                    },
                },
            },
            "Templatable string": {
                "oneOf": [{ "type": "string", "pattern": "^x$" }, { "type": "string" }],
            },
        }));

        assert!(!out.contains("Variants"), "{out}");
    }
}
