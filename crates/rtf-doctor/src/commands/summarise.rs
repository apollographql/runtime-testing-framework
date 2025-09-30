use crate::{format_bytes, new_client};
use anyhow::{Result, anyhow};
use apollo_compiler::executable::{FragmentMap, Selection, SelectionSet};
use clap::ValueEnum;
use rtf_core::graphos::supergraph::{
    SupergraphDetails,
    operations::canned_operations::{schema_with_defer_and_stream, top_studio_canned_ops},
};
use serde::{Deserialize, Serialize};
use tabled::{Table, Tabled, settings::Style};

#[derive(Debug, Clone, Copy, Deserialize, ValueEnum)]
pub enum OpSort {
    /// Sort by size of the raw SDL text in bytes
    Sdl,
    /// Sort by the total number of fields
    Fields,
    /// Sort by the number of fragments
    Fragments,
    /// Sort by the maximum nesting depth
    Depth,
}

pub async fn summarise_graph(
    graph_ref: String,
    n_ops: usize,
    skip_mutations: bool,
    op_sort: Option<OpSort>,
    json_output: bool,
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

    let mut schema_meta = SchemaMeta {
        graph_ref,
        n_types: schema.types.len(),
        sdl_bytes: format_bytes(sg.supergraph_sdl.len()),
        query_resolvers: 0,
        mutation_resolvers: 0,
        subscription_resolvers: 0,
    };

    let roots = [
        (
            &schema.schema_definition.query,
            &mut schema_meta.query_resolvers,
        ),
        (
            &schema.schema_definition.mutation,
            &mut schema_meta.mutation_resolvers,
        ),
        (
            &schema.schema_definition.subscription,
            &mut schema_meta.subscription_resolvers,
        ),
    ];

    for (root, field) in roots {
        let name = match root {
            Some(r) => &r.name,
            None => continue,
        };

        let t = schema.types.get(name).unwrap();
        let obj = t.as_object().unwrap();
        *field = obj.fields.len();
    }

    let mut meta = Meta {
        schema: schema_meta,
        ops: Vec::new(),
    };

    if n_ops > 0 {
        let ops = top_studio_canned_ops(&sg, n_ops, skip_mutations, platform_client).await?;
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
                    raw_sdl_bytes: doc.to_string().len(),
                    n_fields: op.all_fields(doc).count(),
                    n_fragments: doc.fragments.len(),
                    max_depth: max_depth(&op.selection_set, &doc.fragments),
                    request_count: canned_op.request_count,
                    request_count_per_min: canned_op.request_count_per_min,
                }
            })
            .collect();

        if let Some(op_sort) = op_sort {
            match op_sort {
                OpSort::Sdl => op_meta.sort_by_key(|m| m.raw_sdl_bytes),
                OpSort::Fields => op_meta.sort_by_key(|m| m.n_fields),
                OpSort::Fragments => op_meta.sort_by_key(|m| m.n_fragments),
                OpSort::Depth => op_meta.sort_by_key(|m| m.max_depth),
            }

            op_meta.reverse();
        }

        meta.ops = op_meta;
    }

    if json_output {
        println!("{}", serde_json::to_string(&meta)?);
    } else {
        let mut table = Table::new(vec![meta.schema]);
        table.with(Style::markdown());
        println!("{table}");

        if !meta.ops.is_empty() {
            let mut table = Table::new(meta.ops);
            table.with(Style::markdown());
            println!("\n{table}");
        }
    }

    Ok(())
}

#[derive(Serialize)]
struct Meta {
    schema: SchemaMeta,
    ops: Vec<OpMeta>,
}

#[derive(Serialize, Tabled)]
struct SchemaMeta {
    graph_ref: String,
    n_types: usize,
    sdl_bytes: String,
    query_resolvers: usize,
    mutation_resolvers: usize,
    subscription_resolvers: usize,
}

#[derive(Serialize, Tabled)]
struct OpMeta {
    i: usize,
    ty: &'static str,
    sdl_bytes: String,
    #[serde(skip)]
    #[tabled(skip)]
    raw_sdl_bytes: usize,
    n_fields: usize,
    n_fragments: usize,
    max_depth: usize,
    request_count: usize,
    request_count_per_min: usize,
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
