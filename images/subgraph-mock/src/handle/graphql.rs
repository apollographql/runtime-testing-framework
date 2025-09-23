use crate::{
    ADDITIONAL_HEADERS, RESPONSE_GENERATION_CONFIG, SUPERGRAPH_SCHEMA, handle::ByteResponse,
};
use anyhow::anyhow;
use apollo_compiler::{
    ExecutableDocument, Name, Schema,
    ast::OperationType,
    executable::{Selection, SelectionSet},
    schema::ExtendedType,
    validation::Valid,
};
use http_body_util::{BodyExt, Full};
use hyper::{
    Response, StatusCode,
    body::{Bytes, Incoming},
    header::HeaderValue,
};
use rand::{Rng, rngs::ThreadRng, seq::IteratorRandom};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Number, Value, json};
use std::{collections::HashMap, ops::RangeInclusive};
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
) -> anyhow::Result<Value> {
    let op = match doc.operations.get(op_name) {
        Ok(op) => op,
        Err(_) => return Ok(json!({ "data": null })),
    };

    let data = ResponseBuilder::new(
        &mut rand::rng(),
        doc,
        schema,
        RESPONSE_GENERATION_CONFIG.wait(),
    )
    .selection_set(&op.selection_set)?;

    Ok(json!({ "data": data }))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseGenerationConfig {
    pub scalars: HashMap<String, ScalarGenerator>,
    pub array: ArraySize,
    pub null_ratio: Option<(u32, u32)>,
}

impl Default for ResponseGenerationConfig {
    fn default() -> Self {
        let scalars = [
            ("Bool".into(), ScalarGenerator::Bool),
            ("Int".into(), ScalarGenerator::Int { min: 0, max: 100 }),
            ("ID".into(), ScalarGenerator::Int { min: 0, max: 100 }),
            (
                "Float".into(),
                ScalarGenerator::Float {
                    min: -1.0,
                    max: 1.0,
                },
            ),
            (
                "String".into(),
                ScalarGenerator::String {
                    min_len: 1,
                    max_len: 10,
                },
            ),
        ]
        .into_iter()
        .collect();

        Self {
            scalars,
            array: ArraySize {
                min_length: 0,
                max_length: 10,
            },
            null_ratio: Some((1, 2)),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ScalarGenerator {
    Bool,
    Float { min: f64, max: f64 },
    Int { min: i32, max: i32 },
    String { min_len: usize, max_len: usize },
}

impl Default for ScalarGenerator {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl ScalarGenerator {
    const DEFAULT: Self = Self::String {
        min_len: 1,
        max_len: 10,
    };

    fn generate(&self, rng: &mut ThreadRng) -> anyhow::Result<Value> {
        let val = match *self {
            Self::Bool => Value::Bool(rng.random_bool(0.5)),
            Self::Int { min, max } => Value::Number(rng.random_range(min..=max).into()),

            Self::Float { min, max } => Value::Number(
                Number::from_f64(rng.random_range(min..=max)).expect("expected finite float"),
            ),

            // The default Arbitrary impl for String has a random length so we build based on
            // characters instead
            Self::String { min_len, max_len } => {
                let len = rng.random_range(min_len..=max_len);
                // Allow for some multibyte chars. May still need to realloc
                let mut chars = Vec::with_capacity(len * 2);
                for _ in 0..len {
                    chars.push(rng.random::<char>());
                }

                Value::String(chars.into_iter().collect())
            }
        };

        Ok(val)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ArraySize {
    pub min_length: usize,
    pub max_length: usize,
}

impl ArraySize {
    fn range(&self) -> RangeInclusive<usize> {
        self.min_length..=self.max_length
    }
}

struct ResponseBuilder<'a, 'doc, 'schema> {
    rng: &'a mut ThreadRng,
    doc: &'doc Valid<ExecutableDocument>,
    schema: &'schema Valid<Schema>,
    cfg: &'a ResponseGenerationConfig,
}

impl<'a, 'doc, 'schema> ResponseBuilder<'a, 'doc, 'schema> {
    fn new(
        rng: &'a mut ThreadRng,
        doc: &'doc Valid<ExecutableDocument>,
        schema: &'schema Valid<Schema>,
        cfg: &'a ResponseGenerationConfig,
    ) -> Self {
        Self {
            rng,
            doc,
            schema,
            cfg,
        }
    }

    fn selection_set(
        &mut self,
        selection_set: &SelectionSet,
    ) -> anyhow::Result<Map<String, Value>> {
        let mut result = Map::new();

        for selection in &selection_set.selections {
            match selection {
                Selection::Field(field) => {
                    let val = if field.name == "__typename" {
                        Value::String(selection_set.ty.to_string())
                    } else if !field.ty().is_non_null() && self.should_be_null() {
                        Value::Null
                    } else {
                        match (field.selection_set.is_empty(), field.ty().is_list()) {
                            (true, false) => self.leaf_field(field.ty().inner_named_type())?,
                            (true, true) => self.array_leaf_field(field.ty().inner_named_type())?,
                            (false, false) => {
                                Value::Object(self.selection_set(&field.selection_set)?)
                            }
                            (false, true) => {
                                Value::Array(self.array_selection_set(&field.selection_set)?)
                            }
                        }
                    };

                    result.insert(field.name.to_string(), val);
                }

                Selection::FragmentSpread(fragment) => {
                    if let Some(fragment_def) = self.doc.fragments.get(&fragment.fragment_name) {
                        result.extend(self.selection_set(&fragment_def.selection_set)?);
                    }
                }

                Selection::InlineFragment(inline_fragment) => {
                    result.extend(self.selection_set(&inline_fragment.selection_set)?);
                }
            }
        }

        Ok(result)
    }

    fn leaf_field(&mut self, type_name: &Name) -> anyhow::Result<Value> {
        match self.schema.types.get(type_name).unwrap() {
            ExtendedType::Enum(enum_ty) => {
                let enum_value = enum_ty
                    .values
                    .values()
                    .choose(self.rng)
                    .ok_or(anyhow!("empty enum: {type_name}"))?;

                Ok(Value::String(enum_value.value.to_string()))
            }

            ExtendedType::Scalar(scalar) => self
                .cfg
                .scalars
                .get(scalar.name.as_str())
                .unwrap_or(&ScalarGenerator::DEFAULT)
                .generate(self.rng),

            _ => unreachable!("A field with an empty selection set must be a scalar or enum type"),
        }
    }

    fn arbitrary_array_len(&mut self) -> anyhow::Result<usize> {
        Ok(self.rng.random_range(self.cfg.array.range()))
    }

    fn array_selection_set(&mut self, selection_set: &SelectionSet) -> anyhow::Result<Vec<Value>> {
        let num_values = self.arbitrary_array_len()?;
        let mut values = Vec::with_capacity(num_values);
        for _ in 0..num_values {
            values.push(Value::Object(self.selection_set(selection_set)?));
        }

        Ok(values)
    }

    fn array_leaf_field(&mut self, type_name: &Name) -> anyhow::Result<Value> {
        let num_values = self.arbitrary_array_len()?;
        let mut values = Vec::with_capacity(num_values);
        for _ in 0..num_values {
            values.push(self.leaf_field(type_name)?);
        }

        Ok(Value::Array(values))
    }

    fn should_be_null(&mut self) -> bool {
        if let Some((numerator, denominator)) = self.cfg.null_ratio {
            self.rng.random_ratio(numerator, denominator)
        } else {
            false
        }
    }
}
