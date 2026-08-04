//! Reads the caller's identity off the `X-Goog-Authenticated-User-Email` header that Google IAP
//! attaches to every request once it authenticates the caller.
use axum::http::HeaderMap;
use tracing::warn;

const IAP_USER_EMAIL_HEADER: &str = "x-goog-authenticated-user-email";

/// Reads the `X-Goog-Authenticated-User-Email` header (format `prefix:email`, e.g.
/// `accounts.google.com:someone@apollographql.com`) and returns the email.
///
/// Returns `None` if the header is missing, not valid UTF-8, has no colon separator, or has an
/// empty email portion. Every one of those cases is logged at `warn`. This function has no notion
/// of a default/fallback identity — that's a storage concern the caller owns.
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn headers_with(value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(IAP_USER_EMAIL_HEADER, value.parse().unwrap());

        headers
    }

    #[test]
    fn extract_initiator_returns_the_email_portion() {
        let headers = headers_with("accounts.google.com:someone@apollographql.com");

        assert_eq!(
            extract_authenticated_user_email(&headers),
            Some("someone@apollographql.com".to_owned())
        );
    }

    #[test]
    fn extract_initiator_returns_none_when_header_is_missing() {
        let headers = HeaderMap::new();

        assert_eq!(extract_authenticated_user_email(&headers), None);
    }

    #[test]
    fn extract_initiator_returns_none_when_header_has_no_colon() {
        let headers = headers_with("someone@apollographql.com");

        assert_eq!(extract_authenticated_user_email(&headers), None);
    }

    #[test]
    fn extract_initiator_returns_none_when_email_portion_is_empty() {
        let headers = headers_with("accounts.google.com:");

        assert_eq!(extract_authenticated_user_email(&headers), None);
    }

    #[test]
    fn extract_initiator_returns_none_when_header_value_is_not_valid_utf8() {
        let mut headers = HeaderMap::new();
        headers.insert(
            IAP_USER_EMAIL_HEADER,
            HeaderValue::from_bytes(&[0xff, 0xfe]).unwrap(),
        );
        assert_eq!(extract_authenticated_user_email(&headers), None);
    }
}
