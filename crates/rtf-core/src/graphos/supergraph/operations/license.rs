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

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Context;
    use serde::Deserialize;
    use simple_test_case::dir_cases;

    #[dir_cases("crates/rtf-core/resources/test_data/offline_license/valid")]
    #[test]
    fn try_parse_ok(path: &str, contents: &str) -> anyhow::Result<()> {
        let raw: <OfflineLicense as GraphQLQuery>::ResponseData = serde_json::from_str(contents)
            .context(format!("{path} contains malformed test data"))?;

        let jwt = OfflineLicense::try_parse(
            raw,
            offline_license::Variables {
                graph_id: "foo".to_string(),
            },
        )?;

        assert_eq!(jwt, "test-jwt-token");

        Ok(())
    }

    #[derive(Deserialize)]
    struct ErrLicenseCase {
        expected_error: String,
        data: serde_json::Value,
    }

    #[dir_cases("crates/rtf-core/resources/test_data/offline_license/invalid")]
    #[test]
    fn try_parse_err(path: &str, contents: &str) -> anyhow::Result<()> {
        let ErrLicenseCase {
            expected_error,
            data,
        } = serde_json::from_str(contents)
            .context(format!("{path} contains malformed test data"))?;

        let raw: <OfflineLicense as GraphQLQuery>::ResponseData =
            serde_json::from_value(data).context(format!("{path} contains malformed test data"))?;

        let res = OfflineLicense::try_parse(
            raw,
            offline_license::Variables {
                graph_id: "foo".to_string(),
            },
        );

        let cause = match res {
            Ok(_) => panic!("expected error but license was valid"),
            Err(cause) => cause,
        };

        match (expected_error.as_str(), cause) {
            ("UnknownSupergraph", FetchErrorCause::UnknownSupergraph) => (),
            ("UnknownOrganisation", FetchErrorCause::UnknownOrganisation) => (),
            ("OfflineLicenseNotEnabled", FetchErrorCause::OfflineLicenseNotEnabled) => (),
            (err, cause) => panic!("expected {err}, got {cause:?}"),
        }

        Ok(())
    }
}
