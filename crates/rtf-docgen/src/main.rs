use std::{collections::HashMap, fs::File, io::BufReader, path::PathBuf, sync::OnceLock};

use anyhow::{Result, anyhow};
use convert_case::{Boundary, Case, Casing};
use rustdoc_types::{Attribute, Crate, Id, Item, ItemEnum, StructKind, Type, VariantKind};

static FILE_PROVIDER_IDS: OnceLock<Vec<Id>> = OnceLock::new();

fn lookup_item<'idx>(index: &'idx HashMap<Id, Item>, id: &Id) -> Result<&'idx Item> {
    index
        .get(id)
        .ok_or_else(|| anyhow!("FileProvider variant missing data in the doc index"))
}

fn get_docs(item: &Item) -> Result<&String> {
    item.docs
        .as_ref()
        .ok_or_else(|| anyhow!("File Provider variants must be documented"))
}

fn get_name(item: &Item) -> Result<&String> {
    item.name
        .as_ref()
        .ok_or_else(|| anyhow!("File Provider variants must be named"))
}

/// Returns all the IDs for the implementations within the variants of the `FileProvider` enum.
/// These are the concrete top-level items that we want to document.
///
/// Returns [None] if for any reason the data is not formed as we expect and the implementations
/// cannot be found in the index as a result.
fn find_fp_variants(index: &HashMap<Id, Item>) -> Option<Vec<Id>> {
    if let Some((_id, item)) = index.iter().find(|(_id, item)| {
        item.name
            .as_ref()
            .is_some_and(|name| name == "FileProvider")
    }) {
        match &item.inner {
            ItemEnum::Enum(enum_doc) => Some(
                enum_doc
                    .variants
                    .iter()
                    .filter_map(|variant_id| index.get(variant_id))
                    .filter_map(|variant_item| match &variant_item.inner {
                        ItemEnum::Variant(variant) => match &variant.kind {
                            VariantKind::Tuple(ids) => ids.first().and_then(Option::as_ref),
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
            ),
            _ => None,
        }
    } else {
        None
    }
}

/// Writes the table of contents for all the File Providers that will be documented on this page.
///
/// Expects that File Providers will follow the documentation convention of starting their rustdoc
/// comments with a line in the form of `# {human readable name}`.
fn write_table_of_contents(index: &HashMap<Id, Item>) -> Result<()> {
    println!("Available file providers:\n");

    for variant in FILE_PROVIDER_IDS.get().unwrap() {
        let item = lookup_item(index, variant)?;
        println!("{}", generate_toc_link(item)?);
    }
    Ok(())
}

/// Generates the table of contents link for a File Provider
fn generate_toc_link(fp_item: &Item) -> Result<String> {
    let title = get_docs(fp_item)?
        .lines()
        .next()
        .ok_or_else(|| anyhow!("File Provider variants must have a documentation header"))?
        .replace("# ", "");

    let kebab_title = title
        .set_boundaries(&[Boundary::Space])
        .to_case(Case::Kebab);

    Ok(format!("- [{title}](#{kebab_title})"))
}

/// Writes the top-level documentation for each file provider and recursively documents their fields.
fn write_file_provider_docs(index: &HashMap<Id, Item>) -> Result<()> {
    let header = "#";

    for id in FILE_PROVIDER_IDS.get().unwrap() {
        if let Some(item) = index.get(id) {
            if let Ok(docs) = get_docs(item) {
                println!("{header}{docs}\n");
            }

            document_item_content(index, &item.inner, &format!("{header}#"))?
        }
    }
    Ok(())
}

/// Writes out the documentation for a struct's fields or an enum's variants
fn document_item_content(index: &HashMap<Id, Item>, item: &ItemEnum, header: &str) -> Result<()> {
    match item {
        ItemEnum::Struct(inner_struct) => {
            if let StructKind::Plain {
                fields: inner_fields,
                has_stripped_fields: _,
            } = &inner_struct.kind
            {
                if inner_fields.iter().any(|inner_id| {
                    lookup_item(index, inner_id)
                        .ok()
                        .and_then(|inner_item| inner_item.docs.as_ref())
                        .is_some()
                }) {
                    println!("<details>");
                    println!("<summary>Fields</summary>\n");
                    document_inner_fields(index, inner_fields, &format!("{header}#"))?;
                    println!("</details>\n");
                } else {
                    document_inner_fields(index, inner_fields, header)?;
                };
            }
        }
        ItemEnum::Enum(inner_enum) => document_inner_variants(index, &inner_enum.variants, header)?,
        _ => {}
    };
    Ok(())
}

fn document_inner_variants(
    index: &HashMap<Id, Item>,
    inner_variants: &Vec<Id>,
    header: &str,
) -> Result<()> {
    println!("<details>");
    println!("<summary>Variants</summary>\n");

    let mut tuple_variants = Vec::new();
    for inner_id in inner_variants {
        let inner_item = lookup_item(index, inner_id)?;
        let name = get_name(inner_item)?;

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

    for (tuple_variant, meta_items) in tuple_variants {
        let name = get_name(tuple_variant)?;
        if let Some(inner_docs) = &tuple_variant.docs {
            println!("{header}# `{}`\n", name.to_case(Case::Snake));
            println!("{}", inner_docs);
        }
        document_inner_fields(
            index,
            &meta_items.iter().filter_map(|x| *x).collect(),
            &format!("{header}#"),
        )?;
    }

    println!("</details>\n");
    Ok(())
}

fn document_inner_fields(
    index: &HashMap<Id, Item>,
    inner_fields: &Vec<Id>,
    header: &str,
) -> Result<()> {
    for inner_id in inner_fields {
        let inner_item = lookup_item(index, inner_id)?;
        let name = get_name(inner_item)?;

        // Don't write the docs if this was a flattened item or it had no docs
        if let Some(inner_docs) = &inner_item.docs
            && !inner_item
                .attrs
                .contains(&Attribute::Other("#[serde(flatten)]".to_owned()))
        {
            println!("{header} `{name}`\n");
            println!("{inner_docs}\n");
        };

        if let ItemEnum::StructField(Type::ResolvedPath(resolved)) = &inner_item.inner {
            // If we recurse back to a reference to a file provider, just provide a TOC link to it and don't
            // re-document it.
            if FILE_PROVIDER_IDS.get().unwrap().contains(&resolved.id) {
                println!("{}", generate_toc_link(lookup_item(index, &resolved.id)?)?);
            // Only recursively document if:
            //   * the item is not a Field (those should be considered the same as external primitives)
            //   * the item is not an untagged enum variant (an implementation detail that doesn't need to be exposed)
            } else if let Ok(meta_item) = lookup_item(index, &resolved.id)
                && let Some(meta_name) = meta_item.name.as_ref()
                && meta_name != "Field"
                && !meta_item
                    .attrs
                    .contains(&Attribute::Other("#[serde(untagged)]".to_owned()))
            {
                document_item_content(index, &meta_item.inner, header)?;
            }
        }
    }
    Ok(())
}

fn main() -> Result<()> {
    let docfile_path = std::env::args().nth(1).map(PathBuf::from);

    match docfile_path {
        None => Err(anyhow!(
            "A valid rustdoc JSON file path must be provided as the program argument"
        )),
        Some(docfile) => {
            let file = File::open(docfile)?;
            let crate_doc: Crate = serde_json::from_reader(BufReader::new(file))?;
            let index = crate_doc.index;
            let variants = find_fp_variants(&index)
                .filter(|variants| !variants.is_empty())
                .ok_or_else(|| {
                    anyhow!("Could not find the FileProvider enum variants in the docfile")
                })?;
            FILE_PROVIDER_IDS.set(variants).unwrap();

            println!("# File Providers\n");
            write_table_of_contents(&index)?;
            write_file_provider_docs(&index)?;

            Ok(())
        }
    }
}
