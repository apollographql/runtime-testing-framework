use std::collections::HashMap;

use crate::{ADDITIONAL_HEADERS, SUPERGRAPH_SCHEMA, handle::ByteResponse};
use apollo_compiler::{
    ExecutableDocument, Name, Node, Schema,
    ast::{NamedType, OperationType},
    collections::IndexMap,
    executable::{Fragment, Selection, SelectionSet},
    name,
    response::{ExecutionResponse, GraphQLError, JsonMap},
    schema::ExtendedType,
    validation::Valid,
};
use http_body_util::{BodyExt, Full};
use hyper::{
    Response, StatusCode,
    body::{Bytes, Incoming},
    header::HeaderValue,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::{debug, error, trace};

pub async fn handle(body: Incoming) -> anyhow::Result<ByteResponse> {
    let body_bytes = body.collect().await?.to_bytes().to_vec();
    let req: GraphQLRequest = match serde_json::from_slice(&body_bytes) {
        Ok(req) => req,
        Err(err) => {
            error!(%err, "received invalid graphql request");
            let mut resp = Response::new(
                Full::new(err.to_string().into_bytes().into())
                    .map_err(|never| match never {})
                    .boxed(),
            );
            *resp.status_mut() = StatusCode::BAD_REQUEST;

            return Ok(resp);
        }
    };

    let (bytes, status_code) = req.into_response_bytes_and_status_code().await;

    let mut resp = Response::new(Full::new(bytes).map_err(|never| match never {}).boxed());
    *resp.status_mut() = status_code;
    let headers = resp.headers_mut();
    headers.extend(ADDITIONAL_HEADERS.wait().clone());
    headers.insert("Content-Type", HeaderValue::from_static("application/json"));

    Ok(resp)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphQLRequest {
    query: String,
    operation_name: Option<String>,
    #[serde(default)]
    variables: HashMap<String, Value>,
    // #[serde(default)]
    // extensions: serde_json::Map<String, Value>,
}

impl GraphQLRequest {
    #[tracing::instrument(skip(self))]
    pub async fn into_response_bytes_and_status_code(self) -> (Bytes, StatusCode) {
        let schema = SUPERGRAPH_SCHEMA.wait();
        let op_name = self.operation_name.as_deref().unwrap_or("unknown");

        debug!(query=%self.query, "handling graphql request");
        trace!(variables=?self.variables, "request variables");

        let doc = match ExecutableDocument::parse_and_validate(schema, self.query, op_name) {
            Ok(doc) => doc,
            Err(err) => {
                error!("invalid graphql query");
                let errs: Vec<_> = err.errors.iter().map(|d| d.to_json()).collect();
                let bytes = serde_json::to_vec(&json!({ "data": Value::Null, "errors": errs }))
                    .unwrap_or_default();

                return (bytes.into(), StatusCode::BAD_REQUEST);
            }
        };

        // We only accept requests that contain a single operation or a named operation
        let op = match self.operation_name {
            Some(name) => match doc.operations.named.get(&name!(name)) {
                Some(op) => op,
                None => {
                    error!(%name, "unknown operation name");
                    return (Bytes::from("unknown operation"), StatusCode::BAD_REQUEST);
                }
            },

            None => {
                let mut ops = doc.operations.iter();
                let Some(op) = ops.next() else {
                    error!("no operations in graphql request");
                    return (Bytes::from("no operations"), StatusCode::BAD_REQUEST);
                };
                if ops.next().is_some() {
                    error!("multiple operations in graphql request");
                    return (Bytes::from("multiple operations"), StatusCode::BAD_REQUEST);
                }

                op
            }
        };

        debug!(
            name=?op.name,
            type=%op.operation_type,
            n_selections = op.selection_set.selections.len(),
            "processing operation"
        );

        let resp = match op.operation_type {
            OperationType::Query => {
                build_query_response(&op.selection_set, &self.variables, None, 0, &doc, schema)
            }

            OperationType::Subscription => {
                error!("received subscription request: not implemented");
                return (
                    Bytes::from("subscription requests are not implemented"),
                    StatusCode::INTERNAL_SERVER_ERROR,
                );
            }

            OperationType::Mutation => {
                error!("received mutation request: not implemented");
                return (
                    Bytes::from("mutation requests are not implemented"),
                    StatusCode::INTERNAL_SERVER_ERROR,
                );
            }
        };

        match serde_json::to_vec(&resp) {
            Ok(bytes) => (bytes.into(), StatusCode::OK),
            Err(err) => {
                error!(%err, "unable to serialize response");
                (
                    Bytes::from(err.to_string().into_bytes()),
                    StatusCode::INTERNAL_SERVER_ERROR,
                )
            }
        }
    }
}

fn err(msg: impl Into<String>) -> ExecutionResponse {
    ExecutionResponse {
        errors: vec![GraphQLError {
            message: msg.into(),
            locations: Vec::new(),
            path: Vec::new(),
            extensions: Default::default(),
        }],
        data: None,
    }
}

fn build_query_response(
    selset: &SelectionSet,
    vars: &HashMap<String, Value>,
    parent_ty: Option<NamedType>,
    depth: usize,
    doc: &Valid<ExecutableDocument>,
    schema: &Valid<Schema>,
) -> ExecutionResponse {
    let data = JsonMap::with_capacity(selset.selections.len());
    let parent_ty = match parent_ty {
        Some(ty) => match get_parent_ty(ty, doc, schema) {
            Ok(name) => name,
            Err(msg) => return err(msg),
        },
        None => name!("Query"),
    };

    ExecutionResponse {
        data: Some(data),
        errors: Vec::new(),
    }
}

// If the parent type is a union or interface we need to pick a concrete type to work with based on
// what is being queried:
//  - scan through all fragments and collect type conditions
//  - if no type conditions found, just pick one
fn get_parent_ty(
    ty: NamedType,
    doc: &Valid<ExecutableDocument>,
    schema: &Valid<Schema>,
) -> Result<NamedType, String> {
    let parent_ty = schema
        .types
        .get(&ty)
        .ok_or_else(|| format!("{ty} is not a known type"))?;

    match parent_ty {
        ExtendedType::Union(u) => unimplemented!(),
        ExtendedType::Interface(i) => unimplemented!(),
        _ => (),
    }

    Ok(ty)
}

fn get_type_conditions(
    selset: &SelectionSet,
    doc: &ExecutableDocument,
    schema: &Valid<Schema>,
) -> Vec<Name> {
    let mut types = Vec::new();

    for s in selset.selections.iter() {
        match s {
            Selection::Field(_) => continue,

            Selection::InlineFragment(i) => {
                if let Some(ty) = &i.type_condition {
                    types.push(ty.clone());
                }
            }

            Selection::FragmentSpread(f) => match doc.fragments.get(&f.fragment_name) {
                None => error!(name = %f.fragment_name, "unknown fragment"),
                Some(frag) => {
                    let ty = schema.types.get(frag.type_condition()).unwrap();
                    if ty.is_union() || ty.is_interface() {
                        types.extend(get_type_conditions(&frag.selection_set, doc, schema));
                    } else {
                        // This _isn't_ what the Go code does, but surely it should?
                        // it only pushes in the abstract type case
                        types.push(frag.type_condition().clone())
                    }
                }
            },
        }
    }

    types
}
