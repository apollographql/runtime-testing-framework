use anyhow::anyhow;
use apollo_parser::Parser;
use apollo_smith::{Document, DocumentBuilder};
use arbitrary::Unstructured;
use http_body_util::Full;
use hyper::{Request, body::Bytes};
use rand::{RngCore, SeedableRng, rngs::StdRng};
use std::path::PathBuf;
use subgraph_mock::{
    Args,
    handle::{ByteResponse, handle_request},
};

/// Initializes the global state of the mock server based on the optional config file name that maps to
/// a YAML config located in `tests/data/config`.
///
/// If no config file name is provided, the default will be used.
///
/// Returns the port number that the server would have been mapped to, since that value is not actually
/// contained within the state of the app and cannot be otherwise tested as such.
pub fn initialize(config_file_name: Option<&str>) -> anyhow::Result<u16> {
    let pkg_root = env!("CARGO_MANIFEST_DIR");
    let args = Args {
        config: config_file_name
            .map(|name| PathBuf::from(format!("{pkg_root}/tests/data/config/{name}"))),
        schema: PathBuf::from(format!("{pkg_root}/tests/data/schema.graphql")),
    };
    args.init()
}

/// Runs a single request to the mock server through the handler method. Uses seeded RNG for test-case
/// reproducibility.
///
/// If `subgraph_name` is [Some], this request will be sent to the mock as if it were a request to that specific
/// subgraph.
///
/// Ripped pretty much wholesale from the example in the apollo-smith docs.
pub async fn make_request(
    rng_seed: u64,
    subgraph_name: Option<String>,
) -> anyhow::Result<ByteResponse> {
    let pkg_root = env!("CARGO_MANIFEST_DIR");
    let supergraph = std::fs::read_to_string(format!("{pkg_root}/tests/data/schema.graphql"))?;
    let parser = Parser::new(&supergraph);

    let tree = parser.parse();
    if tree.errors().next().is_some() {
        return Err(anyhow!("cannot parse the graphql file"));
    }

    // Convert `apollo_parser::Document` into `apollo_smith::Document`.
    let apollo_smith_doc = Document::try_from(tree.document())?;

    let mut rng = StdRng::seed_from_u64(rng_seed);
    let mut bytes = vec![0u8; 1024];
    rng.fill_bytes(&mut bytes);

    let mut u = Unstructured::new(&bytes);

    // Create a `DocumentBuilder` given an existing document to match a schema.
    let mut gql_doc = DocumentBuilder::with_document(&mut u, apollo_smith_doc)?;
    let operation_def: String = gql_doc.operation_definition()?.unwrap().into();

    let uri = match subgraph_name {
        Some(name) => format!("/{name}"),
        None => "/".to_owned(),
    };

    let req = Request::builder()
        .method("POST")
        .uri(uri)
        .body(Full::new(Bytes::from_owner(operation_def)))?;

    handle_request(req).await
}
