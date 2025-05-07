//! Pull top operation signatures from studio for a given supergraph and rewrite them in order to
//! generate canned graphQL requests that are valid for the parent supergraph schema.
//!
//! The raw operation details pulled from Studio are redacted in a number of ways that cause them
//! to become invalid. See [parse_and_fix] for details of what issues we currently support fixing.
use crate::{
    N_PARALLEL_FETCH,
    platform_query::PlatformQuery,
    supergraph::details::{FetchErrorCause, SupergraphDetails},
};
use anyhow::{Result, bail}; // TODO: replace with thiserror
use apollo_compiler::{
    ExecutableDocument, Name, Node, Schema,
    ast::{self, Argument, DirectiveList, Type},
    executable::{FragmentMap, Operation, Selection, SelectionSet},
    name,
    schema::{DirectiveDefinition, DirectiveLocation, ExtendedType, InputValueDefinition},
    ty,
    validation::Valid,
};
use futures::future::try_join_all;
use graphql_client::GraphQLQuery;
use itertools::Itertools;
use rand::{
    Rng,
    distr::{Alphanumeric, SampleString},
    rngs::ThreadRng,
    seq::IndexedRandom,
};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::Path,
};
use tracing::{debug, error, info, warn};

// Matching the behaviour in rehydrate for now but really this should be configurable
// TODO:
//   - configuring the shape of custom scalars (or just hard coding things like JSON & UUIDs)
//   - passing a seed for RNG to control data generation

/// The number of elements to include in lists (rehydrate is a random number between 2-12)
const RAND_LIST_LEN: usize = 4;
/// Length of generated string fields (and custom scalars that are assumed to be strings)
const RAND_STR_LEN: usize = 20;
/// Length of strings to use for ID fields
const RAND_ID_LEN: usize = 10;
/// Upper bound on Int fields (0..RAND_MAX_INT)
const RAND_MAX_INT: usize = 100;
/// Depth after which we start trying to return empty lists or null where possible
const MAX_NULL_DEPTH: usize = 3;
/// Hard cut off at which we panic to avoid becoming too deeply nested
const MAX_DEPTH: usize = 10;

/// The from field in the filter for fetching operations takes a negative integer as an offest in
/// seconds to set how far back to query operations for. The default behaviour from rehydrate (the
/// go impl used in router-scale) used 24 hours so we match that here.
const LAST_24HOURS_SECONDS: i64 = -(60 * 60 * 24); // TODO: allow customising

/// For a given supergraph, pull the top n operations as reported by studio and generate canned
/// operation data to be able to use them in requests to a running Router.
pub async fn generate_canned_ops(
    details: &SupergraphDetails,
    n: usize,
    skip_mutations: bool,
    api_key: &str,
    staging: bool,
) -> Result<Vec<CannedOperation>> {
    let schema = schema_with_defer_and_stream(&details.supergraph_sdl);
    let signatures = fetch_operation_signatures(
        details.graph_id.clone(),
        details.variant.clone(),
        n,
        skip_mutations,
        api_key,
        staging,
    )
    .await?;

    info!("generating canned operations");
    let mut rng = rand::rng();

    Ok(signatures
        .into_iter()
        .filter_map(
            |sig| match CannedOperation::try_new(sig, &schema, &mut rng) {
                Ok(op) => Some(op),
                Err(e) => {
                    warn!("{e}");
                    None
                }
            },
        )
        .collect())
}

/// The router auto-supports the @defer and @stream directive so schemas may be using them without
/// importing / defining them directly. In that case we need to inject them into the schema in
/// order for the validation of our queries to succeed.
///
/// See https://www.apollographql.com/docs/graphos/routing/operations/defer
///
/// The directive definitions are copied from here:
///   https://github.com/apollographql/router/blob/23e580e22a4401cc2e7a952b241a1ec955b29c99/apollo-federation/src/api_schema.rs#L156https://github.com/apollographql/router/blob/23e580e22a4401cc2e7a952b241a1ec955b29c99/apollo-federation/src/api_schema.rs#L156
fn schema_with_defer_and_stream(sdl: &str) -> Valid<Schema> {
    let mut schema = Schema::parse(sdl, "supergraph.graphql").unwrap();

    // Matching the behaviour in the Router:
    //   https://github.com/apollographql/router/blob/23e580e22a4401cc2e7a952b241a1ec955b29c99/apollo-federation/src/api_schema.rs#L139-L149
    if !schema.directive_definitions.contains_key(&name!("defer")) {
        schema
            .directive_definitions
            .insert(name!("defer"), defer_definition());
    }
    if !schema.directive_definitions.contains_key(&name!("stream")) {
        schema
            .directive_definitions
            .insert(name!("stream"), stream_definition());
    }

    schema.validate().unwrap()
}

fn defer_definition() -> Node<DirectiveDefinition> {
    Node::new(DirectiveDefinition {
        description: None,
        name: name!("defer"),
        arguments: vec![
            Node::new(InputValueDefinition {
                description: None,
                name: name!("label"),
                ty: ty!(String).into(),
                default_value: None,
                directives: Default::default(),
            }),
            Node::new(InputValueDefinition {
                description: None,
                name: name!("if"),
                ty: ty!(Boolean!).into(),
                default_value: Some(true.into()),
                directives: Default::default(),
            }),
        ],
        repeatable: false,
        locations: vec![
            DirectiveLocation::FragmentSpread,
            DirectiveLocation::InlineFragment,
        ],
    })
}

fn stream_definition() -> Node<DirectiveDefinition> {
    Node::new(DirectiveDefinition {
        description: None,
        name: name!("stream"),
        arguments: vec![
            Node::new(InputValueDefinition {
                description: None,
                name: name!("label"),
                ty: ty!(String).into(),
                default_value: None,
                directives: Default::default(),
            }),
            Node::new(InputValueDefinition {
                description: None,
                name: name!("if"),
                ty: ty!(Boolean!).into(),
                default_value: Some(true.into()),
                directives: Default::default(),
            }),
            Node::new(InputValueDefinition {
                description: None,
                name: name!("initialCount"),
                ty: ty!(Int).into(),
                default_value: Some(0.into()),
                directives: Default::default(),
            }),
        ],
        repeatable: false,
        locations: vec![DirectiveLocation::Field],
    })
}

/// Fetch the top `n` operations for a given graph variant and return their "signatures" for
/// processing into valid queries that we can then generate structurally valid variable data for.
async fn fetch_operation_signatures(
    graph_id: String,
    variant: String,
    n: usize,
    skip_mutations: bool,
    api_key: &str,
    staging: bool,
) -> Result<Vec<Signature>> {
    info!("fetching top {n} operation IDs for {graph_id}@{variant}");
    let max_batch_size = 100; // enforced by the studio API
    let batches = n / max_batch_size;
    let mut overflow = n % max_batch_size;
    let mut ids = Vec::with_capacity(n);
    let mut after = None;

    for i in 0..batches {
        info!("requesting batch {}", i + 1);
        let batch = FetchOperationIds::fetch_batch(
            &graph_id,
            &variant,
            skip_mutations,
            max_batch_size as i64,
            after,
            api_key,
            staging,
        )
        .await?;

        let batch_size = batch.ids.len();
        ids.extend(batch.ids);
        after = batch.after;
        if batch_size < max_batch_size {
            overflow = 0;
            break;
        }
    }

    if overflow > 0 {
        info!("requesting batch {}", batches + 1);
        let batch = FetchOperationIds::fetch_batch(
            &graph_id,
            &variant,
            skip_mutations,
            overflow as i64,
            after,
            api_key,
            staging,
        )
        .await?;
        ids.extend(batch.ids);
    }

    if ids.len() < n {
        warn!(
            "ran out of operations in studio: wanted {n} but only able to pull {}",
            ids.len()
        );
    }

    let prev_len = ids.len();
    ids.sort_unstable();
    ids.dedup();

    if ids.len() < prev_len {
        warn!(
            "duplicate operations returned: only have {} unique operations",
            ids.len()
        );
    }

    let mut signatures = Vec::with_capacity(ids.len());

    info!("pulling operation details for {graph_id}@{variant}");
    let mut n_batches = ids.len() / N_PARALLEL_FETCH;
    if n % N_PARALLEL_FETCH > 0 {
        n_batches += 1;
    }
    for (i, batch) in ids
        .into_iter()
        .chunks(N_PARALLEL_FETCH)
        .into_iter()
        .enumerate()
    {
        info!("requesting batch {}/{n_batches}", i + 1);
        let items = try_join_all(
            batch.map(|op_id| Signature::fetch(graph_id.to_string(), op_id, api_key, staging)),
        )
        .await?;
        signatures.extend(items);
    }

    Ok(signatures)
}

/// Declaration for the graphql_client macro code generated from [FetchOperationIds].
pub type Timestamp = i64;

struct Batch {
    ids: Vec<String>,
    after: Option<String>,
}

/// Paginate through the top operations for a given supergraph and time range.
#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "resources/engine-prod-schema.graphql",
    query_path = "resources/queries/fetch-operation-ids.graphql",
    response_derives = "Deserialize",
    variables_derives = "Clone"
)]
struct FetchOperationIds;

impl FetchOperationIds {
    async fn fetch_batch(
        graph_id: &str,
        variant: &str,
        skip_mutations: bool,
        first: i64,
        after: Option<String>,
        api_key: &str,
        staging: bool,
    ) -> Result<Batch, FetchErrorCause> {
        use fetch_operation_ids::OperationType;

        FetchOperationIds::fetch(
            fetch_operation_ids::Variables {
                graph_id: graph_id.to_string(),
                variant: variant.to_string(),
                op_types: if skip_mutations {
                    vec![OperationType::QUERY]
                } else {
                    vec![OperationType::QUERY, OperationType::MUTATION]
                },
                from: LAST_24HOURS_SECONDS,
                first,
                after,
            },
            api_key,
            staging,
        )
        .await
    }
}

impl PlatformQuery for FetchOperationIds {
    type Output = Batch;
    type Error = FetchErrorCause;

    fn try_parse(
        data: Self::ResponseData,
        _: fetch_operation_ids::Variables,
    ) -> Result<Batch, FetchErrorCause> {
        let by_requests = data
            .service
            .ok_or(FetchErrorCause::UnknownSupergraph)?
            .variant
            .ok_or(FetchErrorCause::UnknownVariant)?
            .by_requests;
        let nodes = by_requests.nodes.unwrap_or_default();
        let after = by_requests.page_info.end_cursor;
        let ids: Vec<String> = nodes
            .into_iter()
            .filter(|node| !node.display_name.starts_with("#"))
            .map(|node| node.id)
            .collect();

        Ok(Batch { ids, after })
    }
}

/// Pull the redacted [Signature] for a given operation ID.
#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "resources/engine-prod-schema.graphql",
    query_path = "resources/queries/get-op-signature.graphql",
    response_derives = "Deserialize",
    variables_derives = "Clone"
)]
struct GetOpSignature;

impl PlatformQuery for GetOpSignature {
    type Output = Signature;
    type Error = FetchErrorCause;

    fn try_parse(
        data: Self::ResponseData,
        vars: get_op_signature::Variables,
    ) -> Result<Signature, FetchErrorCause> {
        let sig = data
            .service
            .ok_or(FetchErrorCause::UnknownSupergraph)?
            .operation
            .and_then(|op| op.signature)
            .ok_or(FetchErrorCause::UnknownOperation)?;

        Ok(Signature {
            id: vars.op_id,
            sig,
        })
    }
}

/// A canned operation is a validated graphQL operation from Studio paired with generated data to
/// use as the variables required to send the request to a running Router.
///
/// See [Signature::parse_and_fix] for details on the rewriting we need to do in order to fix up
/// operations pulled from Studio so they are in a form we can work with.
#[derive(Debug, serde::Serialize)]
pub struct CannedOperation {
    /// The platform API ID for this operation in Studio
    pub id: String,
    /// The compact form of this operation that will be used for writing out a JSON POST request
    pub query: String,
    /// The pretty printed form of this operation for writing out a graphl file
    pub pretty_query: String,
    /// The generated variable data for this operation
    pub vars: HashMap<String, Value>,
}

impl CannedOperation {
    /// Attempt to rewrite a [Signature] from Studio into an operation that validates against the
    /// provided [Schema]. If that is successful, generate some randomised variables to pair with
    /// the operation to turn it into a valid request to send to a Router.
    fn try_new(sig: Signature, schema: &Valid<Schema>, rng: &mut ThreadRng) -> Result<Self> {
        debug!("  parsing {}...", sig.id);
        let doc = sig.parse_and_fix(schema, rng)?;
        let mut vars = HashMap::new();

        for op in doc.operations.iter() {
            vars.extend(op.variables.iter().map(|v| {
                let key = v.name.as_str().to_string();
                debug!("    getting data for {key}: {:?}...", v.ty.as_ref());
                let field = FieldSpec::from_ty(v.ty.as_ref(), schema, 0);

                (key, field.as_json(rng))
            }));
        }

        Ok(Self {
            id: sig.id,
            query: doc.serialize().no_indent().to_string(),
            pretty_query: doc.to_string(),
            vars,
        })
    }

    /// Write out both a pretty printed version of the rewritten operation and the JSON payload
    /// required to POST this operation to a running Router.
    ///
    /// The filenames for the written files are `$operationID.graphql` and `$operationID.json`
    /// respectively.
    pub fn write(&self, dir: &Path) -> Result<()> {
        let data = json!({
            "query": self.query,
            "variables": self.vars,
        });

        fs::write(dir.join(format!("{}.graphql", self.id)), &self.pretty_query)?;
        fs::write(
            dir.join(format!("{}.json", self.id)),
            serde_json::to_string_pretty(&data)?,
        )?;

        Ok(())
    }
}

/// A specification for a single field within a [SelectionSet].
///
/// Either wraps an inner [TypeSpec] for concrete data or directly represents an empty list or
/// null value where type information is not required.
#[derive(Debug)]
enum FieldSpec {
    Scalar(TypeSpec),
    List(TypeSpec),
    EmptyList,
    Null,
}

impl FieldSpec {
    /// Use the provided [Type] to build a wrapped [TypeSpec] for a list or scalar.
    ///
    /// [FieldSpec::EmptyList] and [FieldSpec::Null] can just be created directly as no type
    /// information is required in order to serialize them.
    fn from_ty(ty: &Type, schema: &Valid<Schema>, depth: usize) -> Self {
        let spec = TypeSpec::from_named_type(ty.inner_named_type(), schema, depth);
        if ty.is_list() {
            FieldSpec::List(spec)
        } else {
            FieldSpec::Scalar(spec)
        }
    }

    /// Serialize this spec as a serde-json [Value].
    fn as_json(&self, rng: &mut ThreadRng) -> Value {
        match self {
            Self::Scalar(t) => t.as_json(rng),
            Self::List(t) => json!(
                (0..RAND_LIST_LEN)
                    .map(|_| t.as_json(rng))
                    .collect::<Vec<_>>()
            ),
            Self::EmptyList => json!([]),
            Self::Null => json!(null),
        }
    }

    /// Serialize this spec as a apollo_compiler [ast::Value].
    fn as_ast_val(&self, rng: &mut ThreadRng) -> ast::Value {
        match self {
            Self::Scalar(t) => t.as_ast_val(rng),
            Self::List(t) => ast::Value::List(
                (0..RAND_LIST_LEN)
                    .map(|_| Node::new(t.as_ast_val(rng)))
                    .collect(),
            ),
            Self::EmptyList => ast::Value::List(vec![]),
            Self::Null => ast::Value::Null,
        }
    }
}

/// A specification for a given schema type that can be used to generate both JSON data to be
/// included in [CannedOperation] variables and `apollo_compiler` AST data for rewriting an
/// [ExecutableDocument] to ensure that it validates.
#[derive(Debug)]
enum TypeSpec {
    String,
    Int,
    Float,
    Bool,
    Id,
    Enum(Vec<String>),
    Object(HashMap<String, FieldSpec>),
}

impl TypeSpec {
    /// Look up a named type in the provided [Schema] and produce a [TypeSpec] that can be used to
    /// generate randomized data with the correct structure.
    ///
    /// # Panics
    /// This method will panic if passed an invalid type name for the provided schema or if that
    /// type is something other than a scalar, enum or input object.
    fn from_named_type(named_ty: &Name, schema: &Valid<Schema>, depth: usize) -> Self {
        match named_ty.as_str() {
            "String" => return Self::String,
            "Int" => return Self::Int,
            "Float" => return Self::Float,
            "Boolean" => return Self::Bool,
            "Id" => return Self::Id,
            _ => (),
        }

        let schema_ty = schema
            .types
            .get(named_ty)
            .unwrap_or_else(|| panic!("query contained unknown type: {named_ty}"));

        debug!("    schema type: {schema_ty:?}");

        match schema_ty {
            ExtendedType::Scalar(_) => TypeSpec::String, // assume that scalars are strings

            ExtendedType::Enum(t) => TypeSpec::Enum(
                t.values
                    .values()
                    .map(|e| e.value.as_str().to_string())
                    .collect(),
            ),

            ExtendedType::InputObject(obj) => {
                let mut fields = HashMap::with_capacity(obj.fields.len());
                for (name, ivd) in obj.fields.iter() {
                    let field = try_field(named_ty, &ivd.ty, depth, schema);
                    fields.insert(name.as_str().to_string(), field);
                }

                Self::Object(fields)
            }

            t => panic!("unhandled ty: {t}"),
        }
    }

    /// Serialize this spec as a serde-json [Value].
    fn as_json(&self, rng: &mut ThreadRng) -> Value {
        match self {
            Self::String => json!(Alphanumeric.sample_string(rng, RAND_STR_LEN)),
            Self::Int => json!(rng.random_range(0..RAND_MAX_INT)),
            Self::Float => json!(rng.random_range(0.0..1.0)),
            Self::Bool => json!(rng.random_bool(0.5)),
            Self::Id => json!(Alphanumeric.sample_string(rng, RAND_ID_LEN)),
            Self::Enum(variants) => json!(variants.choose(rng).expect("non-empty enum variants")),
            Self::Object(map) => {
                let obj: HashMap<String, Value> = map
                    .iter()
                    .map(|(k, t)| (k.to_string(), t.as_json(rng)))
                    .collect();

                json!(obj)
            }
        }
    }

    /// Serialize this spec as a apollo_compiler [ast::Value].
    fn as_ast_val(&self, rng: &mut ThreadRng) -> ast::Value {
        match self {
            Self::String => ast::Value::from(Alphanumeric.sample_string(rng, RAND_STR_LEN)),
            Self::Int => ast::Value::from(rng.random_range(0..RAND_MAX_INT) as i32),
            Self::Float => ast::Value::from(rng.random_range(0.0..1.0)),
            Self::Bool => ast::Value::from(rng.random_bool(0.5)),
            Self::Id => ast::Value::from(Alphanumeric.sample_string(rng, RAND_ID_LEN)),
            Self::Enum(variants) => ast::Value::Enum(Name::new_unchecked(
                variants
                    .choose(rng)
                    .expect("non-empty enum variants")
                    .as_str(),
            )),
            Self::Object(map) => {
                let obj: Vec<(Name, Node<ast::Value>)> = map
                    .iter()
                    .map(|(k, t)| {
                        (
                            Name::new_unchecked(k.as_str()),
                            Node::new(t.as_ast_val(rng)),
                        )
                    })
                    .collect();

                ast::Value::Object(obj)
            }
        }
    }
}

/// Attempt to recursively build out a valid [FieldSpec] for the provided type while trying to
/// avoid unbounded self referential or deeply nested objects.
///
/// This will panic when `depth` exceeds [MAX_DEPTH].
fn try_field(named_ty: &Name, ty: &Type, depth: usize, schema: &Valid<Schema>) -> FieldSpec {
    // XXX: hack - try to protect against self referential objects
    if ty.inner_named_type() == named_ty || depth > MAX_NULL_DEPTH {
        if ty.is_list() {
            return FieldSpec::EmptyList;
        } else if !ty.is_non_null() {
            return FieldSpec::Null;
        }
    }

    if depth > MAX_DEPTH {
        panic!("exceeded max depth")
    }

    FieldSpec::from_ty(ty, schema, depth + 1)
}

/// The redacted signature for a given graphQL operation stored in Studio.
///
/// The signature itself is a structurally valid graphQL operation but with all hard coded
/// arguments replaced with zero values for their respective types. This mostly leaves the
/// operation in a valid state with the exception of input types which are replaced with empty
/// objects rather than objects containing zeroed fields. See [Signature::parse_and_fix] for
/// details on how this is rewritten into an operation that validates against the parent schema.
#[derive(Debug)]
pub struct Signature {
    id: String,
    sig: String,
}

impl Signature {
    async fn fetch(graph_id: String, id: String, api_key: &str, staging: bool) -> Result<Self> {
        Ok(GetOpSignature::fetch(
            get_op_signature::Variables {
                graph_id,
                op_id: id.clone(),
            },
            api_key,
            staging,
        )
        .await?)
    }

    /// The operations that we pull out of studio using [GetOpSignature] have been partially redacted
    /// to strip any hard coded data present in resolver arguments. As a result, we need to check for
    /// and fix the following known issues before the query will validate (and be usable in a request
    /// to a running Router):
    ///   1. Anywhere there are unused variables we need to remove them.
    ///   2. Conflicting selections without aliases need to have them added.
    ///   3. Input objects with missing required fields are filled out via the same technique used for
    ///      generating variables to acompany the operation.
    fn parse_and_fix(
        &self,
        schema: &Valid<Schema>,
        rng: &mut ThreadRng,
    ) -> Result<Valid<ExecutableDocument>> {
        let mut doc = match ExecutableDocument::parse(schema, &self.sig, &self.id) {
            Ok(doc) => doc,
            Err(with_errs) => {
                let errs: Vec<_> = with_errs.errors.iter().map(|e| e.to_json()).collect();
                let json_errs = serde_json::to_string(&errs)?;
                bail!("error parsing operation: {json_errs}");
            }
        };

        fix_aliases(&mut doc);
        fix_missing_input_fields(&mut doc, schema, rng);
        fix_unused_vars(&mut doc);

        match doc.validate(schema) {
            Ok(doc) => Ok(doc),
            Err(with_errs) => {
                let errs: Vec<_> = with_errs.errors.iter().map(|e| e.to_json()).collect();
                let json_errs = serde_json::to_string(&errs)?;
                bail!("error validating operation: {json_errs}");
            }
        }
    }
}

fn fix_unused_vars(doc: &mut ExecutableDocument) {
    if let Some(operation) = doc.operations.anonymous.as_mut() {
        strip_unused_vars(operation, &doc.fragments);
    }

    for operation in doc.operations.named.values_mut() {
        strip_unused_vars(operation, &doc.fragments);
    }
}

fn fix_aliases(doc: &mut ExecutableDocument) {
    let mut seen = HashSet::new();
    let fragments = doc.fragments.clone();
    let mut suffix = 0;

    for fragment in doc.fragments.values_mut() {
        let frag = fragment.make_mut();
        add_missing_aliases(&mut frag.selection_set, &fragments, &mut suffix, &mut seen);
    }

    if let Some(operation) = doc.operations.anonymous.as_mut() {
        let op = operation.make_mut();
        add_missing_aliases(&mut op.selection_set, &fragments, &mut suffix, &mut seen);
    }

    for operation in doc.operations.named.values_mut() {
        let op = operation.make_mut();
        add_missing_aliases(&mut op.selection_set, &fragments, &mut suffix, &mut seen);
    }
}

fn fix_missing_input_fields(
    doc: &mut ExecutableDocument,
    schema: &Valid<Schema>,
    rng: &mut ThreadRng,
) {
    for fragment in doc.fragments.values_mut() {
        let frag = fragment.make_mut();
        fill_missing_input_fields(&mut frag.selection_set, schema, rng);
    }

    if let Some(operation) = doc.operations.anonymous.as_mut() {
        let op = operation.make_mut();
        fill_missing_input_fields(&mut op.selection_set, schema, rng);
    }

    for operation in doc.operations.named.values_mut() {
        let op = operation.make_mut();
        fill_missing_input_fields(&mut op.selection_set, schema, rng);
    }
}

/// Locate all variables used in field arguments and directives to determine which of the top level
/// operation variables are actually used. Any unused variables are removed from the [Operation].
fn strip_unused_vars(op: &mut Node<Operation>, fragments: &FragmentMap) {
    let mut used_vars = HashSet::new();
    find_used_vars_in_directives(&op.directives, &mut used_vars);
    find_used_vars_in_selset(&op.selection_set, fragments, &mut used_vars);

    let mut unused_vars: Vec<usize> = op
        .variables
        .iter()
        .enumerate()
        .filter_map(|(i, v)| {
            if used_vars.iter().any(|u| *u == v.name.as_str()) {
                None
            } else {
                Some(i)
            }
        })
        .collect();

    unused_vars.reverse();
    let op_ref = op.make_mut();
    for i in unused_vars {
        op_ref.variables.remove(i);
    }
}

#[inline]
fn used_vars_in_args<'a>(args: &'a [Node<Argument>], used_vars: &mut HashSet<&'a str>) {
    for arg in args.iter() {
        used_vars_from_val(arg.value.as_ref(), used_vars);
    }
}

#[inline]
fn used_vars_from_val<'a>(val: &'a ast::Value, used_vars: &mut HashSet<&'a str>) {
    match val {
        ast::Value::Variable(var) => {
            used_vars.insert(var.as_str());
        }
        ast::Value::Object(fields) => {
            for (_, val) in fields {
                used_vars_from_val(val, used_vars);
            }
        }
        _ => (),
    }
}

#[inline]
fn find_used_vars_in_directives<'a>(
    directives: &'a DirectiveList,
    used_vars: &mut HashSet<&'a str>,
) {
    for d in directives.iter() {
        used_vars_in_args(&d.arguments, used_vars);
    }
}

fn find_used_vars_in_selset<'a>(
    selset: &'a SelectionSet,
    fragments: &'a FragmentMap,
    used_vars: &mut HashSet<&'a str>,
) {
    for sel in selset.selections.iter() {
        match sel {
            Selection::Field(f) => {
                used_vars_in_args(&f.arguments, used_vars);
                find_used_vars_in_directives(&f.directives, used_vars);
                find_used_vars_in_selset(&f.selection_set, fragments, used_vars);
            }

            Selection::FragmentSpread(spread) => {
                let f = match fragments.get(&spread.fragment_name) {
                    Some(f) => f,
                    None => {
                        warn!("spread of unknown fragment name: {}", spread.fragment_name);
                        continue;
                    }
                };

                find_used_vars_in_directives(&spread.directives, used_vars);
                find_used_vars_in_directives(&f.directives, used_vars);
                find_used_vars_in_selset(&f.selection_set, fragments, used_vars);
            }

            Selection::InlineFragment(f) => {
                find_used_vars_in_directives(&f.directives, used_vars);
                find_used_vars_in_selset(&f.selection_set, fragments, used_vars)
            }
        }
    }
}

/// Add in aliases for any repeated field resolvers within the provided [SelectionSet].
///
/// See usage in [Signature::parse_and_fix]: we require that all fragmments have been pre-processed
/// in order to reserve their field names when they are spread into selection sets.
fn add_missing_aliases(
    selset: &mut SelectionSet,
    fragments: &FragmentMap,
    suffix: &mut usize,
    seen: &mut HashSet<Name>,
) {
    // Note all of the existing names/aliases coming from fragment spreads first so we correctly
    // update inline fields with new aliases if they would clash with fields coming from a
    // fragment.
    // -> This requires that all of the fragments have already been pre-processed as part of
    //    [Signature::parse_and_fix].
    for sel in selset.selections.iter() {
        if let Selection::FragmentSpread(s) = sel {
            if let Some(frag) = fragments.get(&s.fragment_name) {
                for sel in frag.selection_set.selections.iter() {
                    if let Selection::Field(f) = sel {
                        let name = match f.alias.as_ref() {
                            Some(name) => name.clone(),
                            None => f.name.clone(),
                        };
                        seen.insert(name);
                    }
                }
            }
        }
    }

    for sel in selset.selections.iter_mut() {
        match sel {
            Selection::FragmentSpread(_) => (), // handled above

            Selection::Field(field) => {
                if field.name == "__typename" {
                    continue;
                }

                let f = field.make_mut();
                match f.alias.clone() {
                    Some(alias) => {
                        seen.insert(alias);
                    }
                    None => {
                        // If we've already encountered this field name previously then we need to add
                        // an alias for all subsequent ocurrences.
                        if seen.contains(&f.name) {
                            f.alias = Some(Name::new_unchecked(&format!("alias_{}", *suffix)));
                            *suffix += 1;
                        } else {
                            seen.insert(f.name.clone());
                        }
                    }
                }

                // Reset the set of seen field names when we move down to the selection set on
                // child fields in order to avoid adding aliases we don't need
                let mut seen = HashSet::new();
                add_missing_aliases(&mut f.selection_set, fragments, suffix, &mut seen);
            }

            Selection::InlineFragment(f) => {
                add_missing_aliases(&mut f.make_mut().selection_set, fragments, suffix, seen)
            }
        }
    }
}

/// Locatate and stub out any empty input objects found within the provided [SelectionSet] so that
/// all required fields are present.
fn fill_missing_input_fields(
    selset: &mut SelectionSet,
    schema: &Valid<Schema>,
    rng: &mut ThreadRng,
) {
    for sel in selset.selections.iter_mut() {
        match sel {
            Selection::FragmentSpread(_) => (), // handled explicitly in parse_and_fix

            Selection::Field(field) => {
                let f = field.make_mut();
                for arg in f.arguments.iter_mut() {
                    if matches!(arg.value.as_ref(), ast::Value::Object(o) if o.is_empty()) {
                        let ty = match f.definition.argument_by_name(arg.name.as_str()) {
                            Some(ivd) => ivd.ty.as_ref(),
                            None => {
                                error!("unknown field argument: {}", arg.name.as_str());
                                continue;
                            }
                        };
                        let val = FieldSpec::from_ty(ty, schema, 0).as_ast_val(rng);
                        arg.make_mut().value = Node::new(val);
                    }
                }

                fill_missing_input_fields(&mut f.selection_set, schema, rng);
            }

            Selection::InlineFragment(f) => {
                fill_missing_input_fields(&mut f.make_mut().selection_set, schema, rng)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use simple_test_case::dir_cases;

    const SCHEMA: &str = include_str!("../../../resources/engine-prod-schema.graphql");

    #[test]
    fn find_used_vars_in_selset_works() {
        let q = include_str!("../../../resources/test_data/queries/query_with_unused_vars.graphql");
        let schema = Schema::parse_and_validate(SCHEMA, "supergraph.graphql").unwrap();

        let doc = ExecutableDocument::parse(&schema, q, "test").unwrap();
        let op = doc.operations.named.values().next().unwrap();
        let mut vars: Vec<&str> = op.variables.iter().map(|v| v.name.as_str()).collect();
        vars.sort_unstable();
        assert_eq!(
            vars,
            vec![
                "bar",
                "foo",
                "graph_id",
                "include_id",
                "include_variants",
                "op_id",
                "op_types"
            ],
            "initial"
        );

        let mut used_vars = HashSet::new();
        find_used_vars_in_selset(&op.selection_set, &doc.fragments, &mut used_vars);
        let mut used_vars: Vec<_> = used_vars.into_iter().collect();
        used_vars.sort_unstable();

        assert_eq!(
            used_vars,
            vec![
                "graph_id",
                "include_id",
                "include_variants",
                "op_id",
                "op_types"
            ],
            "used"
        );
    }

    #[test]
    fn fix_unused_vars_works() {
        let q = include_str!("../../../resources/test_data/queries/query_with_unused_vars.graphql");
        let schema = Schema::parse_and_validate(SCHEMA, "supergraph.graphql").unwrap();
        let mut doc = ExecutableDocument::parse(&schema, q, "test").unwrap();

        let res = doc.clone().validate(&schema);
        assert!(res.is_err(), "doc should have been invalid");

        let op = doc.operations.named.values_mut().next().unwrap();
        let mut vars: Vec<&str> = op.variables.iter().map(|v| v.name.as_str()).collect();
        vars.sort_unstable();
        assert_eq!(
            vars,
            vec![
                "bar",
                "foo",
                "graph_id",
                "include_id",
                "include_variants",
                "op_id",
                "op_types"
            ],
            "before"
        );

        fix_unused_vars(&mut doc);

        let op = doc.operations.named.values_mut().next().unwrap();
        let mut vars: Vec<&str> = op.variables.iter().map(|v| v.name.as_str()).collect();
        vars.sort_unstable();
        assert_eq!(
            vars,
            vec![
                "graph_id",
                "include_id",
                "include_variants",
                "op_id",
                "op_types"
            ],
            "after"
        );

        let res = doc.validate(&schema);
        assert!(res.is_ok(), "{res:?}");
    }

    #[test]
    fn fix_aliases_works() {
        let q =
            include_str!("../../../resources/test_data/queries/query_requiring_aliases.graphql");
        let schema = Schema::parse_and_validate(SCHEMA, "supergraph.graphql").unwrap();
        let mut doc = ExecutableDocument::parse(&schema, q, "test").unwrap();

        let res = doc.clone().validate(&schema);
        assert!(res.is_err(), "doc should have been invalid");

        fix_aliases(&mut doc);

        let res = doc.validate(&schema);
        assert!(res.is_ok(), "{res:?}");
    }

    #[test]
    fn fix_missing_input_fields_works() {
        let q = include_str!(
            "../../../resources/test_data/queries/query_with_missing_input_fields.graphql"
        );
        let schema = Schema::parse_and_validate(SCHEMA, "supergraph.graphql").unwrap();
        let mut doc = ExecutableDocument::parse(&schema, q, "test").unwrap();

        let res = doc.clone().validate(&schema);
        assert!(res.is_err(), "doc should have been invalid");

        let mut rng = rand::rng();
        fix_missing_input_fields(&mut doc, &schema, &mut rng);

        let res = doc.validate(&schema);
        assert!(res.is_ok(), "{res:?}");
    }

    #[dir_cases("crates/rtf-core/resources/test_data/queries")]
    #[test]
    fn parse_and_fix_works(path: &str, contents: &str) {
        let schema = Schema::parse_and_validate(SCHEMA, "supergraph.graphql").unwrap();
        let sig = Signature {
            id: path.to_string(),
            sig: contents.to_string(),
        };

        let mut rng = rand::rng();
        let res = sig.parse_and_fix(&schema, &mut rng);

        assert!(res.is_ok(), "{res:?}");
    }
}
