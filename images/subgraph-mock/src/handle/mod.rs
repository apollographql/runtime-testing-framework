use crate::LATENCY_GENERATOR;
use http_body_util::{BodyExt, Full, combinators::BoxBody};
use hyper::{
    Method, Request, Response, StatusCode,
    body::{Bytes, Incoming},
};
use tokio::time::{Instant, sleep};
use tracing::{trace, warn};

mod graphql;

type ByteResponse = Response<BoxBody<Bytes, hyper::Error>>;

/// Top level handler function that is called for every incoming request from Hyper.
pub async fn handle_request(req: Request<Incoming>) -> anyhow::Result<ByteResponse> {
    let (parts, body) = req.into_parts();
    let (method, path) = (parts.method, parts.uri.path());

    let res = match (&method, path) {
        (&Method::POST, "/") => graphql::handle(body).await,

        // default to 404
        (method, path) => {
            warn!(%method, %path, "received unexpected request");
            let mut resp = Response::new(
                Full::new("Not found\n".into())
                    .map_err(|never| match never {})
                    .boxed(),
            );
            *resp.status_mut() = StatusCode::NOT_FOUND;

            Ok(resp)
        }
    };

    // Skip latency injection when we have an internal server error
    if res.is_ok() {
        let latency = LATENCY_GENERATOR.wait().generate(Instant::now());
        trace!(latency_ms = latency.as_millis(), "injecting latency");
        sleep(latency).await;
    }

    res
}
