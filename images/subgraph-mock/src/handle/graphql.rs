use crate::{ADDITIONAL_HEADERS, SUPERGRAPH_SCHEMA, handle::ByteResponse};
use apollo_compiler::{ExecutableDocument, Schema, ast::OperationType, validation::Valid};
use apollo_smith::{ResponseBuilder, Unstructured};
use http_body_util::{BodyExt, Full};
use hyper::{
    Response, StatusCode,
    body::{Bytes, Incoming},
    header::HeaderValue,
};
use rand::Rng;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
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

        let doc = match ExecutableDocument::parse_and_validate(schema, &self.query, op_name) {
            Ok(doc) => doc,
            Err(err) => {
                let errs: Vec<_> = err.errors.iter().map(|d| d.to_json()).collect();
                error!(?errs, query=%self.query, "invalid graphql query");
                let bytes = serde_json::to_vec(&json!({ "data": Value::Null, "errors": errs }))
                    .unwrap_or_default();
                return (bytes.into(), StatusCode::BAD_REQUEST);
            }
        };

        let op = doc.operations.iter().next().unwrap();
        let op_name = op.name.as_ref().map(|name| name.as_str());

        debug!(
            ?op_name,
            type=%op.operation_type,
            n_selections = op.selection_set.selections.len(),
            "processing operation"
        );

        let resp = match op.operation_type {
            OperationType::Query => match generate_response(op_name, &doc, schema) {
                Ok(resp) => resp,
                Err(err) => {
                    error!(%err, "unable to generate response");
                    return (
                        Bytes::from("unable to generate response"),
                        StatusCode::INTERNAL_SERVER_ERROR,
                    );
                }
            },

            // Not currently supporting mutations or subscriptions
            op_type => {
                error!("received {op_type} request: not implemented");
                return (
                    Bytes::from("not implemented"),
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

fn generate_response(
    op_name: Option<&str>,
    doc: &Valid<ExecutableDocument>,
    schema: &Valid<Schema>,
) -> anyhow::Result<apollo_smith::Value> {
    let mut buf = [0u8; 2048];
    rand::rng().fill(&mut buf);
    let mut u = Unstructured::new(&buf);

    // TODO: null ratio, min/max list size
    let resp = ResponseBuilder::new(&mut u, doc, schema)
        .with_operation_name(op_name)
        .with_null_ratio(1, 2)
        .build()?;

    Ok(resp)
}
