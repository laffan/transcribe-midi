//! The one HTTP client, shared by every provider.
//!
//! Split out when a second provider arrived: the headers and the URL differ between
//! Anthropic and an OpenAI-compatible server, and nothing else does. `ureq` is blocking
//! and pure Rust; the tool loop already runs on its own thread, so an async runtime would
//! buy nothing.
//!
//! The transport is **Apple-only**, for two reasons that happen to agree. The feature is
//! Apple-only — off-Apple there is no Keychain to hold a key — and `rustls`'s crypto
//! backends need a C compiler for the Apple targets, which the Linux build host does not
//! have, so pulling one in would cost us the cross-compile check that catches API
//! breakage before it reaches a Mac. `native-tls` on macOS and iOS is Security.framework
//! through pure-Rust bindings: no C to build, and certificate verification follows the
//! system trust store rather than a root list baked into the binary.
//!
//! Everything that *interprets* a response is platform-independent and tested, because
//! that is the part with logic in it.

use serde_json::Value;

use crate::error::AiError;

/// A response, reduced to the two things any caller cares about.
pub struct HttpResponse {
    pub status: u16,
    pub body: Value,
}

/// Headers as `(name, value)`. A slice rather than a map: there are two or three of them
/// and their order is the order they were written in.
pub type Headers<'a> = &'a [(&'a str, String)];

#[cfg(any(target_os = "macos", target_os = "ios"))]
mod transport {
    use std::time::Duration;

    use serde_json::Value;
    use ureq::tls::{RootCerts, TlsConfig, TlsProvider};

    use super::{Headers, HttpResponse};
    use crate::error::AiError;

    /// Generous, because a request with extended thinking can legitimately take minutes
    /// and the alternative is a timeout the user reads as a bug. A local model on a busy
    /// machine is no faster.
    const REQUEST_TIMEOUT: Duration = Duration::from_secs(300);

    fn agent() -> ureq::Agent {
        ureq::Agent::config_builder()
            .timeout_global(Some(REQUEST_TIMEOUT))
            // Read the body on a 4xx: the service's own error message is the most useful
            // thing it can tell us, and a bare status code throws it away.
            .http_status_as_error(false)
            .tls_config(
                TlsConfig::builder()
                    .provider(TlsProvider::NativeTls)
                    // Security.framework's own roots, so an enterprise or MDM trust
                    // policy applies here as it does everywhere else on the device.
                    .root_certs(RootCerts::PlatformVerifier)
                    .build(),
            )
            .build()
            .new_agent()
    }

    fn finish(
        result: Result<ureq::http::Response<ureq::Body>, ureq::Error>,
    ) -> Result<HttpResponse, AiError> {
        let mut response = result.map_err(|e| AiError::Transport(e.to_string()))?;
        let status = response.status().as_u16();
        let body: Value = response
            .body_mut()
            .read_json()
            .map_err(|e| AiError::Protocol(format!("the response was not JSON: {e}")))?;
        Ok(HttpResponse { status, body })
    }

    pub(super) fn get(url: &str, headers: Headers<'_>) -> Result<HttpResponse, AiError> {
        let mut request = agent().get(url);
        for (name, value) in headers {
            request = request.header(*name, value);
        }
        finish(request.call())
    }

    pub(super) fn post(
        url: &str,
        headers: Headers<'_>,
        body: &Value,
    ) -> Result<HttpResponse, AiError> {
        let mut request = agent().post(url).header("content-type", "application/json");
        for (name, value) in headers {
            request = request.header(*name, value);
        }
        finish(request.send_json(body))
    }
}

#[cfg(not(any(target_os = "macos", target_os = "ios")))]
mod transport {
    use serde_json::Value;

    use super::{Headers, HttpResponse};
    use crate::error::AiError;

    fn unavailable() -> AiError {
        AiError::Transport("AI editing needs the macOS or iOS build".into())
    }

    pub(super) fn get(_url: &str, _headers: Headers<'_>) -> Result<HttpResponse, AiError> {
        Err(unavailable())
    }

    pub(super) fn post(
        _url: &str,
        _headers: Headers<'_>,
        _body: &Value,
    ) -> Result<HttpResponse, AiError> {
        Err(unavailable())
    }
}

pub fn get(url: &str, headers: Headers<'_>) -> Result<HttpResponse, AiError> {
    transport::get(url, headers)
}

pub fn post(url: &str, headers: Headers<'_>, body: &Value) -> Result<HttpResponse, AiError> {
    transport::post(url, headers, body)
}

/// Either the JSON body, or a typed API error carrying the service's own wording.
///
/// Both shapes are handled because they are both common: Anthropic and OpenAI put the
/// text under `error.message`, and a local server that is merely unhappy often returns a
/// bare `error` string.
pub fn interpret(response: HttpResponse) -> Result<Value, AiError> {
    if (200..300).contains(&response.status) {
        return Ok(response.body);
    }

    let message = response.body["error"]["message"]
        .as_str()
        .or_else(|| response.body["error"].as_str())
        .or_else(|| response.body["message"].as_str())
        .map(str::to_owned)
        .unwrap_or_else(|| response.body.to_string());

    Err(AiError::Api { status: response.status, message })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_success_yields_the_body() {
        let body = json!({"ok": true});
        let out = interpret(HttpResponse { status: 200, body: body.clone() }).unwrap();
        assert_eq!(out, body);
    }

    #[test]
    fn an_error_keeps_the_services_own_wording() {
        let error = interpret(HttpResponse {
            status: 401,
            body: json!({"error": {"message": "invalid x-api-key"}}),
        })
        .unwrap_err();

        match error {
            AiError::Api { status, message } => {
                assert_eq!(status, 401);
                assert_eq!(message, "invalid x-api-key");
            }
            other => panic!("wrong error: {other:?}"),
        }
    }

    #[test]
    fn a_bare_error_string_is_read_too() {
        // What a local server returns when it is unhappy but not an API.
        let error = interpret(HttpResponse {
            status: 404,
            body: json!({"error": "Model not loaded"}),
        })
        .unwrap_err();

        match error {
            AiError::Api { message, .. } => assert_eq!(message, "Model not loaded"),
            other => panic!("wrong error: {other:?}"),
        }
    }

    #[test]
    fn an_error_with_no_message_still_says_something() {
        let error = interpret(HttpResponse { status: 500, body: json!({"weird": 1}) }).unwrap_err();
        match error {
            AiError::Api { status, message } => {
                assert_eq!(status, 500);
                assert!(message.contains("weird"), "the body is better than nothing");
            }
            other => panic!("wrong error: {other:?}"),
        }
    }
}
