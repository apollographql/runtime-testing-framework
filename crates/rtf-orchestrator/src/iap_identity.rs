//! Reads the caller's identity off the `X-Goog-Authenticated-User-Email` header that Google IAP
//! attaches to every request once it authenticates the caller.
use axum::http::HeaderMap;
use rtf_integrations::orchestrator::TRIGGER_REPO_HEADER;
use tracing::warn;

pub const IAP_USER_EMAIL_HEADER: &str = "x-goog-authenticated-user-email";

/// Reads the `X-Goog-Authenticated-User-Email` header (format `prefix:email`, e.g.
/// `accounts.google.com:someone@apollographql.com`) and returns the email.
pub fn extract_authenticated_user_email(headers: &HeaderMap) -> Option<String> {
    headers
        .get(IAP_USER_EMAIL_HEADER)
        .or_else(|| {
            warn!("{IAP_USER_EMAIL_HEADER} header missing from request");

            None
        })
        .and_then(|value| {
            value
                .to_str()
                .inspect_err(|err| warn!(%err, "{IAP_USER_EMAIL_HEADER} header is not valid UTF-8"))
                .ok()
        })
        .and_then(|value| {
            value.split_once(':').or_else(|| {
                warn!("{IAP_USER_EMAIL_HEADER} header has no colon separator");

                None
            })
        })
        .map(|(_, email)| email)
        .filter(|email| {
            let is_empty = email.is_empty();
            if is_empty {
                warn!("{IAP_USER_EMAIL_HEADER} header has an empty email portion");
            }

            !is_empty
        })
        .map(|email| email.to_owned())
}

/// Parse the `X-Rtf-Trigger-Repo` header (format `org/repo`) and split it into its org and repo
/// components.
/// We only pay attention to this for known automation users which are already verified via the IAP
/// header so we simply trust the value that has been set.
pub fn try_extract_trigger_repo(headers: &HeaderMap) -> Option<(String, String)> {
    headers
        .get(TRIGGER_REPO_HEADER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split_once('/'))
        .filter(|(org, repo)| !org.is_empty() && !repo.is_empty() && !repo.contains('/'))
        .map(|(org, repo)| (org.to_owned(), repo.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;
    use simple_test_case::test_case;

    #[test]
    fn extract_initiator_returns_none_when_header_is_missing() {
        assert_eq!(extract_authenticated_user_email(&HeaderMap::new()), None);
    }

    #[test_case(b"accounts.google.com:someone@apollographql.com", Some("someone@apollographql.com"); "valid")]
    #[test_case(b"someone@apollographql.com", None; "missing prefix before the colon")]
    #[test_case(b"accounts.google.com:", None; "missing the user email")]
    #[test_case(&[0xff, 0xfe], None; "invalid utf8")]
    #[test]
    fn extract_initiator_returns_expected_data(header_value_bytes: &[u8], expected: Option<&str>) {
        let mut headers = HeaderMap::new();
        headers.insert(
            IAP_USER_EMAIL_HEADER,
            HeaderValue::from_bytes(header_value_bytes).unwrap(),
        );

        assert_eq!(
            extract_authenticated_user_email(&headers),
            expected.map(|s| s.to_string())
        );
    }

    #[test]
    fn extract_trigger_repo_returns_none_when_header_is_missing() {
        assert_eq!(try_extract_trigger_repo(&HeaderMap::new()), None);
    }

    #[test_case(b"foo-org/bar-repo", Some(("foo-org", "bar-repo")); "valid")]
    #[test_case(b"foo-org", None; "no slash")]
    #[test_case(b"/bar-repo", None; "no org before slash")]
    #[test_case(b"foo-org/bar-repo/baz-path", None; "more than one slash")]
    #[test_case(&[0xff, 0xfe], None; "invalid utf8")]
    #[test]
    fn extract_trigger_repo_returns_expected_data(
        header_value_bytes: &[u8],
        expected: Option<(&str, &str)>,
    ) {
        let mut headers = HeaderMap::new();
        headers.insert(
            TRIGGER_REPO_HEADER,
            HeaderValue::from_bytes(header_value_bytes).unwrap(),
        );

        assert_eq!(
            try_extract_trigger_repo(&headers),
            expected.map(|(org, repo)| (org.to_string(), repo.to_string()))
        );
    }
}
