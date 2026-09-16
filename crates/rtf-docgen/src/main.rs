use anyhow::{Result, anyhow, bail};
use convert_case::{Boundary, Case, Casing};
use rustdoc_types::{Attribute, Crate, Id, Item, ItemEnum, StructKind, Type, VariantKind};
use std::{collections::HashMap, env, fs};

fn main() -> Result<()> {
    let path = env::args()
        .nth(1)
        .ok_or_else(|| anyhow!("no rustdoc JSON file path provided"))?;

    let krate: Crate = serde_json::from_slice(&fs::read(path)?)?;
    let writer = DocWriter::new(krate.index)?;

    println!("# File Providers\n");
    writer.write_table_of_contents()?;
    writer.write_file_provider_docs()
}

struct DocWriter {
    fp_ids: Vec<Id>,
    index: HashMap<Id, Item>,
}

impl DocWriter {
    fn new(index: HashMap<Id, Item>) -> Result<Self> {
        let (_, fp_item) = index
            .iter()
            .find(|(_, item)| {
                item.name
                    .as_ref()
                    .is_some_and(|name| name == "FileProvider")
            })
            .ok_or_else(|| anyhow!("Could not find the FileProvider enum in rustdoc output"))?;

        let fp_ids = match &fp_item.inner {
            ItemEnum::Enum(enum_declaration) => enum_declaration
                .variants
                .iter()
                .filter_map(|variant_id| index.get(variant_id))
                .filter_map(|variant_item| match &variant_item.inner {
                    ItemEnum::Variant(variant) => match &variant.kind {
                        VariantKind::Tuple(ids) => ids.first().and_then(|opt| opt.as_ref()),
                        _ => None,
                    },
                    _ => None,
                })
                .filter_map(|inner_id| index.get(inner_id))
                .filter_map(|inner_item| match &inner_item.inner {
                    ItemEnum::StructField(Type::ResolvedPath(path)) => Some(path.id),
                    _ => None,
                })
                .collect(),

            it => bail!("FileProvider should be an Enum, found {:?}", it.item_kind()),
        };

        Ok(Self { fp_ids, index })
    }

    /// Writes the table of contents for all the File Providers that will be documented on this page.
    ///
    /// Expects that File Providers will follow the documentation convention of starting their rustdoc
    /// comments with a line in the form of `# {human readable name}`.
    fn write_table_of_contents(&self) -> Result<()> {
        println!("Available file providers:\n");

        for variant in self.fp_ids.iter() {
            let item = self.lookup_item(variant)?;
            println!("{}", self.generate_toc_link(item)?);
        }

        Ok(())
    }

    /// Writes the top-level documentation for each file provider and recursively documents their fields.
    fn write_file_provider_docs(&self) -> Result<()> {
        let header = "#";

        for id in self.fp_ids.iter() {
            if let Some(item) = self.index.get(id) {
                if let Ok(docs) = item_docs(item) {
                    println!("{header}{docs}\n");
                }

                self.document_item_content(&item.inner, &format!("{header}#"))?
            }
        }

        Ok(())
    }

    fn lookup_item(&self, id: &Id) -> Result<&Item> {
        self.index
            .get(id)
            .ok_or_else(|| anyhow!("FileProvider variant missing data in the doc index"))
    }

    fn generate_toc_link(&self, fp_item: &Item) -> Result<String> {
        let title = item_docs(fp_item)?
            .lines()
            .next()
            .ok_or_else(|| anyhow!("File Provider variants must have a documentation header"))?
            .replace("# ", "");

        let kebab_title = title
            .set_boundaries(&[Boundary::Space])
            .to_case(Case::Kebab);

        Ok(format!("- [{title}](#{kebab_title})"))
    }

    /// Writes out the documentation for a struct's fields or an enum's variants
    fn document_item_content(&self, item: &ItemEnum, header: &str) -> Result<()> {
        match item {
            ItemEnum::Struct(inner_struct) => {
                if let StructKind::Plain {
                    fields,
                    has_stripped_fields: _,
                } = &inner_struct.kind
                {
                    if fields.iter().any(|inner_id| {
                        self.lookup_item(inner_id)
                            .ok()
                            .and_then(|inner_item| inner_item.docs.as_ref())
                            .is_some()
                    }) {
                        println!("<details>");
                        println!("<summary>Fields</summary>\n");
                        self.document_inner_fields(fields, &format!("{header}#"))?;
                        println!("</details>\n");
                    } else {
                        self.document_inner_fields(fields, header)?;
                    };
                }
            }

            ItemEnum::Enum(inner_enum) => {
                self.document_inner_variants(&inner_enum.variants, header)?
            }

            _ => (),
        }

        Ok(())
    }

    fn document_inner_fields(&self, inner_fields: &[Id], header: &str) -> Result<()> {
        for inner_id in inner_fields.iter() {
            let inner_item = self.lookup_item(inner_id)?;
            let name = item_name(inner_item)?;

            let is_flattened = inner_item
                .attrs
                .contains(&Attribute::Other("#[serde(flatten)]".to_owned()));

            // Don't write the docs if this was a flattened item whose target will be documented via
            // recursion below, or if it had no docs
            if let Some(inner_docs) = &inner_item.docs
                && !(is_flattened && self.flattened_field_recurses(inner_item))
            {
                println!("{header} `{name}`\n");
                println!("{inner_docs}\n");
            };

            if let ItemEnum::StructField(Type::ResolvedPath(resolved)) = &inner_item.inner {
                // If we recurse back to a reference to a file provider, just provide a TOC link to it and don't
                // re-document it.
                if self.fp_ids.contains(&resolved.id) {
                    let fp_item = self.lookup_item(&resolved.id)?;
                    println!("{}", self.generate_toc_link(fp_item)?);
                    continue;
                }

                if let Ok(fp_item) = self.lookup_item(&resolved.id)
                    && self.is_not_field_or_untagged_enum_variant(fp_item)
                {
                    self.document_item_content(&fp_item.inner, header)?;
                }
            }
        }

        Ok(())
    }

    fn document_inner_variants(&self, inner_variants: &Vec<Id>, header: &str) -> Result<()> {
        println!("<details>");
        println!("<summary>Variants</summary>\n");

        let mut tuple_variants = Vec::new();
        for inner_id in inner_variants {
            let inner_item = self.lookup_item(inner_id)?;
            let name = item_name(inner_item)?;

            // Tuple variants are written out as maps in the YAML and should be documented
            // as their full list of supported values after the list of named variants
            if let ItemEnum::Variant(variant) = &inner_item.inner
                && let VariantKind::Tuple(meta_items) = &variant.kind
            {
                tuple_variants.push((inner_item, meta_items));
            } else {
                print!("- `{}`", name.to_case(Case::Snake));
                if let Some(inner_docs) = &inner_item.docs {
                    print!(": {}", inner_docs.replace("\n", "  "));
                }

                println!();
            }
        }

        println!("\n");

        for (tuple_variant, meta_items) in tuple_variants.into_iter() {
            let name = item_name(tuple_variant)?;
            if let Some(inner_docs) = &tuple_variant.docs {
                println!("{header}# `{}`\n", name.to_case(Case::Snake));
                println!("{}", inner_docs);
            }
            self.document_inner_fields(
                &meta_items.iter().filter_map(|x| *x).collect::<Vec<_>>(),
                &format!("{header}#"),
            )?;
        }

        println!("</details>\n");

        Ok(())
    }

    /// Returns `true` if documenting a flattened field's [ResolvedPath][Type::ResolvedPath] target
    /// would recurse into further documentation (a TOC link or a nested struct/enum). Flattened
    /// fields that don't resolve to anything documentable (e.g. a `HashMap`) have no replacement
    /// content, so their own doc comment must still be written out.
    fn flattened_field_recurses(&self, inner_item: &Item) -> bool {
        let resolved = match &inner_item.inner {
            ItemEnum::StructField(Type::ResolvedPath(resolved)) => resolved,
            _ => return false,
        };

        self.fp_ids.contains(&resolved.id)
            || self
                .lookup_item(&resolved.id)
                .is_ok_and(|item| self.is_not_field_or_untagged_enum_variant(item))
    }

    // Whether or not this item is:
    //   - not a Field (those should be considered the same as external primitives)
    //   - not an untagged enum variant (an implementation detail that doesn't need to be exposed)
    fn is_not_field_or_untagged_enum_variant(&self, fp_item: &Item) -> bool {
        fp_item.name.as_deref().is_some_and(|meta_name| {
            meta_name != "Field"
                && !fp_item
                    .attrs
                    .contains(&Attribute::Other("#[serde(untagged)]".to_owned()))
        })
    }
}

fn item_docs(item: &Item) -> Result<&String> {
    item.docs
        .as_ref()
        .ok_or_else(|| anyhow!("File Provider variants must be documented"))
}

fn item_name(item: &Item) -> Result<&String> {
    item.name
        .as_ref()
        .ok_or_else(|| anyhow!("File Provider variants must be named"))
}
