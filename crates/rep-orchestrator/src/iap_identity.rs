//! Reads the caller's identity off the `X-Goog-Authenticated-User-Email` header that Google IAP
//! attaches to every request once it authenticates the caller.

use axum::http::HeaderMap;

const IAP_USER_EMAIL_HEADER: &str = "x-goog-authenticated-user-email";
const UNKNOWN_USER: &str = "unknown";

/// Reads the `X-Goog-Authenticated-User-Email` header (format `prefix:email`, e.g.
/// `accounts.google.com:someone@apollographql.com`) and returns the email.
///
/// Returns `unknown` if the Header is missing, invalid utf-8 or there is no email to extract
pub fn extract_initiator(headers: &HeaderMap) -> &str {
    match headers.get(IAP_USER_EMAIL_HEADER) {
        Some(user) => {
            let user_str = match user.to_str() {
                Ok(str) => str,
                Err(_) => return UNKNOWN_USER,
            };

            let email = user_str.split_once(":").map(|(_, email)| email);

            match email {
                Some(email) => {
                    if email.is_empty() {
                        return UNKNOWN_USER;
                    }

                    email
                }
                None => UNKNOWN_USER,
            }
        }
        None => UNKNOWN_USER,
    }
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

        assert_eq!(extract_initiator(&headers), "someone@apollographql.com");
    }

    #[test]
    fn extract_initiator_returns_unknown_when_header_is_missing() {
        let headers = HeaderMap::new();

        assert_eq!(extract_initiator(&headers), UNKNOWN_USER);
    }

    #[test]
    fn extract_initiator_returns_unknown_when_header_has_no_colon() {
        let headers = headers_with("someone@apollographql.com");

        assert_eq!(extract_initiator(&headers), UNKNOWN_USER);
    }

    #[test]
    fn extract_initiator_returns_unknown_when_email_portion_is_empty() {
        let headers = headers_with("accounts.google.com:");

        assert_eq!(extract_initiator(&headers), UNKNOWN_USER);
    }

    #[test]
    fn extract_initiator_returns_unknown_when_header_value_is_not_valid_utf8() {
        let mut headers = HeaderMap::new();
        headers.insert(
            IAP_USER_EMAIL_HEADER,
            HeaderValue::from_bytes(&[0xff, 0xfe]).unwrap(),
        );
        assert_eq!(extract_initiator(&headers), UNKNOWN_USER);
    }
}
