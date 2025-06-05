//! Gernerated graphQL queries for the Apollo platform API using the graphql-client crate.
//!
//! The docs for the internal platform API can be found here in studio:
//!   <https://studio-staging.apollographql.com/graph/engine/variant/prod/home>
//!
//! The supergraph can be found in the resources directory at the root of this crate.
//!
//! See here for docs on how to add new queries:
//!   <https://github.com/graphql-rust/graphql-client?tab=readme-ov-file#getting-started>
use crate::ReqwestClient;
use graphql_client::GraphQLQuery;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use tracing::{error, trace};

/// API endpoint for the staging studio instance.
/// -> Apollo internal graphs are queried from here
pub(crate) const STAGING_STUDIO_URL: &str =
    "https://graphql-staging.api.apollographql.com/api/graphql";
/// API endpoint for the production studio instance.
/// -> Customer graphs are queried from here
pub(crate) const PROD_STUDIO_URL: &str = "https://graphql.api.apollographql.com/api/graphql";

/// Error variants that we can encounter when making graphQL requests to the platform API.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The operation being run returned graphQL errors
    #[error(
        "graphql errors returned when running operation: {:?}",
        .errors.iter().map(|e| &e.message).collect::<Vec<_>>()
    )]
    GraphqlErrors {
        /// The raw graphQL errors that were returned
        errors: Vec<GqlError>,
    },

    /// The operation being run returned unexpected graphQL extensions
    #[error(
        "unexpected graphql extensions returned running operation: {:?}",
        .extensions.iter().map(|(k, v)| format!("{k}: {v:?}")).collect::<Vec<_>>()
    )]
    GraphqlExtensions {
        /// The raw graphQL extensions returned with the response for the operation
        extensions: serde_json::Map<String, serde_json::Value>,
    },

    /// No data or errors were returned from the platform API in response to running an operation.
    #[error("no data returned from graphql operation")]
    NoData,

    // Wrapped errors
    /// An underlying error from the reqwest crate
    #[error(transparent)]
    Reqwest(#[from] reqwest::Error),

    /// An underlying error from the serde-json crate
    #[error(transparent)]
    SerdeJson(#[from] serde_json::Error),
}

/// Serialization format for graphQL errors: <https://spec.graphql.org/October2021/#sec-Errors.Error-result-format>
#[derive(Debug, Deserialize)]
pub struct GqlError {
    /// The string error message
    pub message: String,
    /// The locations within the operation being run where the errors ocurred
    #[serde(default)]
    pub locations: Option<Vec<BTreeMap<String, u32>>>,
    /// The path through the provided operation to the location of the error
    #[serde(default)]
    pub path: Option<Vec<serde_json::Value>>,
}

/// Helper trait for packaging up making a graphql request to the Apollo platform API and parsing the
/// response into a more ergonomic type.
///
/// # Deriving GraphQLQuery
/// Types that implement this trait need to derive the [GraphQLQuery] trait from [graphql_client]
/// first. The schema file for the platform API (`engine@prod`) along with the query files used to
/// define new queries are located in the top level `resources` directory of this crate. The details
/// on how to work with the graphql_client crate are covered in [their docs][0] but there are couple
/// of things we need to specify as boiler plate in order for the [PlatformQuery] trait to work:
///   - variables need to derive [Clone].
///   - responses need to derive [Deserialize].
///   - The name of the query in your graphql file needs to exactly match the name of the Rust
///     struct
///
/// The macro will then generate a module with a name that is the snake_case transfrom of your
/// struct (so `MyQuery` becomes `my_query`) that contains all of the generated types needed to
/// make your request and parse the response.
///
/// # Example
/// ```graphql
/// # contents of queries/my-query.graphql
/// query MyQuery(
///   $my_arg: String!,
///   $my_other_arg: Int,
///   $my_enums: [MyEnum]!
/// ) {
///   foo(arg: $my_arg) {
///     bar(arg: $my_other_arg, enums: $my_enums) {
///       myData # a nullable string
///      }
///   }
/// }
/// ```
///
/// ```ignore
/// #[derive(GraphQLQuery)]
/// #[graphql(
///     schema_path = "resources/engine-prod-schema.graphql",
///     query_path = "queries/my-query.graphql",
///     response_derives = "Deserialize",
///     variables_derives = "Clone"
/// )]
/// pub struct MyQuery;
///
/// impl PlatformQuery for MyQuery {
///     type Output = String;
///     type Error = &'static str;
///
///     fn try_parse(
///         data: Self::ResponseData,
///         _vars: my_query::Variables
///     ) -> Result<String, &'static str> {
///         // The nested structure of Self::ResponseData matches the your graphQL query
///         let maybe_my_data = data.foo.bar.my_data;
///
///         match maybe_my_data {
///             Some(s) => Ok(s),
///             None => Err("myData was null"),
///         }
///     }
/// }
///
/// // Making a request
/// let vars = my_query::Variables {
///     my_arg: "some arg".to_string(),      // non-nullable types in graphql map to Rust types
///     my_other_arg: Some(42),              // nullable types map to options
///     my_enums: vec![my_query::MyEnum::A], // enums are available in the generated module
/// };
/// let api_key = "...";
/// let client = ReqwestClient::new_prod(api_key);
///
/// let parsed_response = MyQuery::fetch(vars, &client).await?;
/// ```
///
///   [0]: https://github.com/graphql-rust/graphql-client?tab=readme-ov-file#getting-started
#[allow(async_fn_in_trait)]
pub trait PlatformQuery: GraphQLQuery + Sized
where
    Self::Variables: Clone,
{
    /// The output type returned from `try_parse`
    type Output;
    /// The error type returned from `try_parse`
    type Error: From<Error>;

    /// Attempt to parse the raw data returned from a graphQL query into a usable Rust type.
    fn try_parse(
        data: Self::ResponseData,
        variables: Self::Variables,
    ) -> Result<Self::Output, Self::Error>;

    /// Execute this query and parse the returned data
    async fn fetch(
        variables: Self::Variables,
        client: &impl Client,
    ) -> Result<Self::Output, Self::Error> {
        let raw = client.execute_operation::<Self>(variables.clone()).await?;

        Self::try_parse(raw, variables)
    }
}

/// An API client that can make requests to the Apollo platform API.
#[allow(async_fn_in_trait)]
pub trait Client {
    /// POST a GraphQL operation to the studio API with appropriate headers, returning the raw JSON
    /// response.
    ///
    /// [Client::execute_operation] is used to handle creating the POST body and parsing the
    /// response.
    async fn post_operation(&self, body: &impl Serialize) -> Result<serde_json::Value, Error>;

    /// Helper for making an API request to studio and handling any serialization or graphQL errors so
    /// that implementations of [PlatformQuery::try_parse] only need to care about mapping valid data.
    async fn execute_operation<T>(&self, variables: T::Variables) -> Result<T::ResponseData, Error>
    where
        T: GraphQLQuery,
    {
        let body = T::build_query(variables);
        let raw = self.post_operation(&body).await?;

        trace!("raw response: {raw:?}");
        let resp: GqlResponse<T::ResponseData> = serde_json::from_value(raw)?;

        if !resp.errors.is_empty() {
            error!("Errors returned when running graphql operation");
            return Err(Error::GraphqlErrors {
                errors: resp.errors,
            });
        } else if !resp.extensions.is_empty() {
            error!("Unexpected extensions returned when running graphql operation");
            return Err(Error::GraphqlExtensions {
                extensions: resp.extensions,
            });
        }

        return resp.data.ok_or(Error::NoData);

        // Serde type for parsing the graphql response from the server

        #[derive(Debug, Deserialize)]
        struct GqlResponse<T> {
            data: Option<T>,
            #[serde(default)]
            errors: Vec<GqlError>,
            #[serde(default)]
            extensions: serde_json::Map<String, serde_json::Value>,
        }
    }
}

impl Client for ReqwestClient {
    async fn post_operation(&self, body: &impl Serialize) -> Result<serde_json::Value, Error> {
        let raw = self
            .inner
            .post(&self.platform.url)
            .json(body)
            .header("x-api-key", &self.platform.api_key)
            .header("apollo-sudo", "true")
            .header("apollographql-client-name", "runtime-testing-framework")
            .header("apollographql-client-version", "0.1.0")
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?;

        Ok(raw)
    }
}
