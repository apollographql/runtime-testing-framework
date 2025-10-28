use std::{collections::HashMap, fs::File, io::BufReader, path::PathBuf};

use anyhow::{Result, anyhow};
use convert_case::{Boundary, Case, Casing};
use rustdoc_types::{Crate, Id, Item, ItemEnum, StructKind, Type, VariantKind};

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
fn write_table_of_contents(index: &HashMap<Id, Item>, variants: &Vec<Id>) -> Result<()> {
    println!("Available file providers:\n");

    for variant in variants {
        let item = lookup_item(index, variant)?;

        let title = get_docs(item)?
            .lines()
            .next()
            .ok_or_else(|| anyhow!("File Provider variants must have a documentation header"))?
            .replace("# ", "");

        let kebab_title = title
            .with_boundaries(&[Boundary::SPACE])
            .to_case(Case::Kebab);

        println!("- [{title}](#{kebab_title})");
    }
    Ok(())
}

/// Writes the documentation for each variant and its fields. Does not include documentation
/// for fields of fields at this time.
fn write_struct_docs(index: &HashMap<Id, Item>, struct_ids: &Vec<Id>) -> Result<()> {
    for id in struct_ids {
        let item = lookup_item(index, id)?;

        let docs = get_docs(item)?;
        println!("#{docs}\n");

        match &item.inner {
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
                        println!("### Fields\n");
                    }

                    // TODO: recurse on inner fields if they are other known types within the crate
                    for inner_id in inner_fields {
                        let inner_item = lookup_item(index, inner_id)?;
                        let name = get_name(inner_item)?;

                        if let Some(inner_docs) = &inner_item.docs {
                            println!("#### `{name}`\n");
                            println!("{inner_docs}\n");
                        }
                    }
                }
            }
            _ => {
                // TODO: support enums so we can recurse here
            }
        };
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

            println!("# File Providers\n");
            write_table_of_contents(&index, &variants)?;
            write_struct_docs(&index, &variants)?;

            Ok(())
        }
    }
}
