use crate::{format_bytes, new_client};
use anyhow::{Result, anyhow};
use apollo_compiler::{
    Schema,
    executable::{FragmentMap, Selection, SelectionSet},
    validation::Valid,
};
use clap::ValueEnum;
use graphql_client::GraphQLQuery;
use rtf_core::graphos::{
    PlatformClient,
    platform_query::PlatformQuery,
    supergraph::{
        SupergraphDetails,
        operations::canned_operations::{schema_with_defer_and_stream, top_studio_canned_ops},
    },
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
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
    /// Sort by the number of entity types included in the operation
    Entities,
}

pub async fn summarise_launch(
    graph_ref: String,
    launch_id: String,
    json_output: bool,
) -> Result<()> {
    let (graph_id, variant) = graph_ref
        .split_once('@')
        .ok_or(anyhow!("invalid graph ref"))?;

    let client = new_client();
    let platform_client = client.platform_client().ok_or(anyhow!(
        "no API credentials provided for making Apollo platform requests"
    ))?;

    let sg = LaunchById::fetch(
        launch_by_id::Variables {
            graph_id: graph_id.to_string(),
            variant: variant.to_string(),
            launch_id,
        },
        platform_client,
    )
    .await?;

    let meta = SchemaMeta::try_new(&sg).await?;

    if json_output {
        println!("{}", serde_json::to_string(&meta)?);
    } else {
        let mut table = Table::new(vec![meta]);
        table.with(Style::markdown());
        println!("{table}");
    }

    Ok(())
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
    let mut meta = Meta {
        schema: SchemaMeta::try_new(&sg).await?,
        ops: Vec::new(),
    };

    if n_ops > 0 {
        let mut ops = top_studio_canned_ops(&sg, n_ops, skip_mutations, platform_client).await?;
        ops.sort_unstable_by_key(|op| op.request_count);
        ops.reverse();

        let entities = find_entities(&sg.graph_id, &sg.variant, platform_client).await?;

        let mut op_meta: Vec<_> = ops
            .into_iter()
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

                let (max_depth, op_entities) =
                    max_depth_and_entities(&op.selection_set, &doc.fragments, &entities);

                OpMeta {
                    i: i + 1,
                    id: canned_op.id,
                    ty,
                    sdl_bytes: format_bytes(doc.to_string().len()),
                    raw_sdl_bytes: doc.to_string().len(),
                    fields: op.all_fields(doc).count(),
                    fragments: doc.fragments.len(),
                    entities: op_entities.len(),
                    max_depth,
                    request_count: canned_op.request_count,
                    request_count_per_min: canned_op.request_count_per_min,
                }
            })
            .collect();

        if let Some(op_sort) = op_sort {
            match op_sort {
                OpSort::Sdl => op_meta.sort_by_key(|m| m.raw_sdl_bytes),
                OpSort::Fields => op_meta.sort_by_key(|m| m.fields),
                OpSort::Fragments => op_meta.sort_by_key(|m| m.fragments),
                OpSort::Depth => op_meta.sort_by_key(|m| m.max_depth),
                OpSort::Entities => op_meta.sort_by_key(|m| m.entities),
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
    types: usize,
    entities: usize,
    interfaces: usize,
    sdl_bytes: String,
    subgraphs: usize,
    queries: usize,
    mutations: usize,
    subscriptions: usize,
}

impl SchemaMeta {
    async fn try_new(sg: &SupergraphDetails) -> Result<Self> {
        let schema = schema_with_defer_and_stream(&sg.supergraph_sdl);
        let entities = count_entities(&schema);
        let sdl_bytes = schema.to_string().len();

        let mut schema_meta = SchemaMeta {
            graph_ref: format!("{}@{}", sg.graph_id, sg.variant),
            types: schema.types.len(),
            entities,
            interfaces: count_interfaces(&schema),
            sdl_bytes: format_bytes(sdl_bytes),
            subgraphs: count_subgraphs(&schema),
            queries: 0,
            mutations: 0,
            subscriptions: 0,
        };

        let roots = [
            (&schema.schema_definition.query, &mut schema_meta.queries),
            (
                &schema.schema_definition.mutation,
                &mut schema_meta.mutations,
            ),
            (
                &schema.schema_definition.subscription,
                &mut schema_meta.subscriptions,
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

        Ok(schema_meta)
    }
}

#[derive(Serialize, Tabled)]
struct OpMeta {
    i: usize,
    id: String,
    ty: &'static str,
    sdl_bytes: String,
    #[serde(skip)]
    #[tabled(skip)]
    raw_sdl_bytes: usize,
    fields: usize,
    fragments: usize,
    entities: usize,
    max_depth: usize,
    request_count: usize,
    request_count_per_min: usize,
}

fn max_depth_and_entities(
    selset: &SelectionSet,
    fragments: &FragmentMap,
    entities: &HashSet<String>,
) -> (usize, HashSet<String>) {
    let mut max = 0;
    let mut all_entities = HashSet::new();

    for sel in selset.selections.iter() {
        let (m, sel_entities) = match sel {
            Selection::FragmentSpread(s) => {
                if let Some(f) = fragments.get(&s.fragment_name) {
                    max_depth_and_entities(&f.selection_set, fragments, entities)
                } else {
                    (1, HashSet::new())
                }
            }

            Selection::Field(f) => {
                if f.selection_set.is_empty() {
                    (1, HashSet::new())
                } else {
                    let ty = f.definition.ty.inner_named_type().as_str();
                    if entities.contains(ty) {
                        all_entities.insert(ty.to_string());
                    }

                    max_depth_and_entities(&f.selection_set, fragments, entities)
                }
            }

            Selection::InlineFragment(f) => {
                max_depth_and_entities(&f.selection_set, fragments, entities)
            }
        };

        all_entities.extend(sel_entities);
        if m > max {
            max = m;
        }
    }

    (max + 1, all_entities)
}

fn count_entities(schema: &Valid<Schema>) -> usize {
    schema
        .types
        .values()
        .filter(|ty| {
            ty.directives()
                .iter()
                .any(|d| d.name == "join__type" && d.arguments.iter().any(|arg| arg.name == "key"))
        })
        .count()
}

fn count_interfaces(schema: &Valid<Schema>) -> usize {
    schema.types.values().filter(|ty| ty.is_interface()).count()
}

fn count_subgraphs(schema: &Valid<Schema>) -> usize {
    if let Some(ty) = schema.types.get("join__Graph")
        && let Some(e) = ty.as_enum()
    {
        e.values.len()
    } else {
        0
    }
}

async fn find_entities(
    graph_id: &str,
    variant: &str,
    client: &PlatformClient,
) -> Result<HashSet<String>> {
    FetchEntities::fetch(
        fetch_entities::Variables {
            graph_id: graph_id.into(),
            variant: variant.into(),
        },
        client,
    )
    .await
}

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "../rtf-core/resources/engine-prod-schema.graphql",
    query_path = "resources/queries/fetch_entities.graphql",
    response_derives = "Deserialize",
    variables_derives = "Clone"
)]
pub struct FetchEntities;

impl PlatformQuery for FetchEntities {
    type Output = HashSet<String>;
    type Error = anyhow::Error;

    fn try_parse(
        data: Self::ResponseData,
        _vars: fetch_entities::Variables,
    ) -> Result<HashSet<String>> {
        use fetch_entities::FetchEntitiesGraphVariantEntities::*;

        let entities = data
            .graph
            .ok_or(anyhow!("unknown graph"))?
            .variant
            .ok_or(anyhow!("unknown variant"))?
            .entities
            .ok_or(anyhow!("unable to query entities"))?;

        match entities {
            EntitiesResponse(inner) => Ok(inner.entities.into_iter().map(|e| e.typename).collect()),
            EntitiesErrorResponse => Err(anyhow!("error fetching entities")),
        }
    }
}

type GraphQLDocument = String;

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "../rtf-core/resources/engine-prod-schema.graphql",
    query_path = "resources/queries/launch_by_id.graphql",
    response_derives = "Deserialize",
    variables_derives = "Clone"
)]
pub struct LaunchById;

impl PlatformQuery for LaunchById {
    type Output = SupergraphDetails;
    type Error = anyhow::Error;

    fn try_parse(
        data: Self::ResponseData,
        vars: launch_by_id::Variables,
    ) -> Result<SupergraphDetails> {
        use launch_by_id::LaunchByIdGraphVariantLaunchBuildResult as BuildRes;

        let build_result = data
            .graph
            .ok_or(anyhow!("unknown graph"))?
            .variant
            .ok_or(anyhow!("unknown variant"))?
            .launch
            .ok_or(anyhow!("unknown launch"))?
            .build
            .and_then(|b| b.result)
            .ok_or(anyhow!("no build"))?;

        let supergraph_sdl = match build_result {
            BuildRes::BuildFailure => return Err(anyhow!("not a successful build")),
            BuildRes::BuildSuccess(res) => res.core_schema.core_document,
        };

        if supergraph_sdl.is_empty() {
            return Err(anyhow!("empty supergraph schema"));
        }

        Ok(SupergraphDetails {
            variant: vars.variant,
            graph_id: vars.graph_id,
            supergraph_sdl,
            subgraphs: Vec::new(),
        })
    }
}
