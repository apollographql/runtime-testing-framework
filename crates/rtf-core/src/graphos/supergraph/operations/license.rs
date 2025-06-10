//! Attempt to fetch an offline GraphOS license
use crate::graphos::{
    platform_query::{self, PlatformQuery},
    supergraph::{FetchError, FetchErrorCause},
};
use graphql_client::GraphQLQuery;

/// Attempt to fetch an offline GraphOS license for the given supergraph
pub async fn fetch_offline_license(
    graph_id: impl Into<String>,
    client: &impl platform_query::Client,
) -> Result<String, FetchError> {
    let graph_id = graph_id.into();
    let vars = offline_license::Variables {
        graph_id: graph_id.clone(),
    };

    OfflineLicense::fetch(vars, client)
        .await
        .map_err(|cause| FetchError {
            graph_id,
            variant: String::new(),
            cause,
        })
}

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "resources/engine-prod-schema.graphql",
    query_path = "resources/queries/offline-license.graphql",
    response_derives = "Deserialize",
    variables_derives = "Clone"
)]
struct OfflineLicense;

impl PlatformQuery for OfflineLicense {
    type Output = String;
    type Error = FetchErrorCause;

    fn try_parse(
        data: Self::ResponseData,
        _vars: Self::Variables,
    ) -> Result<Self::Output, Self::Error> {
        let jwt = data
            .graph
            .ok_or(FetchErrorCause::UnknownSupergraph)?
            .account
            .ok_or(FetchErrorCause::UnknownOrganisation)?
            .offline_license
            .ok_or(FetchErrorCause::OfflineLicenseNotEnabled)?
            .jwt;

        Ok(jwt)
    }
}
