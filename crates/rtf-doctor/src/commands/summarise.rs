use crate::{format_bytes, new_client};
use anyhow::{Result, anyhow};
use apollo_compiler::executable::{FragmentMap, Selection, SelectionSet};
use rtf_core::graphos::supergraph::{
    SupergraphDetails,
    operations::top_studio_operations::{generate_canned_ops, schema_with_defer_and_stream},
};
use tabled::{Table, Tabled, settings::Style};

pub async fn summarise_graph(
    graph_ref: &str,
    n_ops: usize,
    skip_mutations: bool,
    by_fields: bool,
) -> Result<()> {
    let (graph_id, variant) = graph_ref
        .split_once('@')
        .ok_or(anyhow!("invalid graph ref"))?;

    let client = new_client();
    let platform_client = client.platform_client().ok_or(anyhow!(
        "no API credentials provided for making Apollo platform requests"
    ))?;

    let sg = SupergraphDetails::fetch(graph_id, variant, platform_client).await?;
    let schema = schema_with_defer_and_stream(&sg.supergraph_sdl);

    println!(":: Schema details");
    println!("n types: {}", schema.types.len());
    println!("SDL bytes: {}", format_bytes(sg.supergraph_sdl.len()));

    let roots = [
        ("query", &schema.schema_definition.query),
        ("mutation", &schema.schema_definition.mutation),
        ("subscription", &schema.schema_definition.subscription),
    ];

    println!("schema roots:");
    for (name, root) in roots {
        let name = match root {
            Some(r) => &r.name,
            None => {
                println!("  no {name} root");
                continue;
            }
        };

        let t = schema.types.get(name).unwrap();
        let obj = t.as_object().unwrap();
        println!("  {} {name} resolvers", obj.fields.len());
    }

    if n_ops == 0 {
        return Ok(());
    }

    let ops = generate_canned_ops(&sg, n_ops, skip_mutations, platform_client).await?;
    let mut op_meta: Vec<_> = ops
        .iter()
        .enumerate()
        .map(|(i, canned_op)| {
            let doc = &canned_op.doc;
            let op = doc.operations.iter().next().unwrap();
            let ty = if op.is_query() {
                "query"
            } else if op.is_mutation() {
                "mutation"
            } else if op.is_subscription() {
                "subscription"
            } else {
                "unknown"
            };

            OpMeta {
                i: i + 1,
                ty,
                sdl_bytes: format_bytes(doc.to_string().len()),
                n_fields: op.all_fields(doc).count(),
                n_fragments: doc.fragments.len(),
                max_depth: max_depth(&op.selection_set, &doc.fragments),
            }
        })
        .collect();

    if by_fields {
        op_meta.sort_by_key(|m| m.n_fields);
        op_meta.reverse();
    }

    println!(":: Top operation details");
    let mut table = Table::new(op_meta);
    table.with(Style::markdown());
    println!("{table}");

    Ok(())
}

#[derive(Tabled)]
struct OpMeta {
    i: usize,
    ty: &'static str,
    sdl_bytes: String,
    n_fields: usize,
    n_fragments: usize,
    max_depth: usize,
}

fn max_depth(selset: &SelectionSet, fragments: &FragmentMap) -> usize {
    let mut max = 0;

    for sel in selset.selections.iter() {
        let m = match sel {
            Selection::FragmentSpread(s) => {
                if let Some(f) = fragments.get(&s.fragment_name) {
                    max_depth(&f.selection_set, fragments) + 1
                } else {
                    1
                }
            }

            Selection::Field(f) => {
                if f.selection_set.is_empty() {
                    1
                } else {
                    max_depth(&f.selection_set, fragments) + 1
                }
            }

            Selection::InlineFragment(f) => max_depth(&f.selection_set, fragments) + 1,
        };

        if m > max {
            max = m;
        }
    }

    max
}
