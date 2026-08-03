//! Anthropic / Claude-compatible JSON extractor / response.
//!
//! Drop-in alternative to [`axum::Json`] for Claude-compatible routes.
//! When request-body extraction fails, the rejection is an Anthropic-shaped
//! JSON error instead of Axum's plain-text
//! [`JsonRejection`](axum::extract::rejection::JsonRejection).
//!
//! ```json
//! {
//!   "type": "error",
//!   "error": {
//!     "type": "invalid_request_error",
//!     "message": "Failed to parse the request body as JSON: …"
//!   }
//! }
//! ```

use axum::extract::{FromRequest, Request};
use axum::http::header::{self, HeaderMap, HeaderValue};
use axum::response::{IntoResponse, Response};
use bytes::{BufMut, Bytes, BytesMut};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::error::{AppError, ClaudeError};

/// JSON extractor and response type that rejects with Anthropic error bodies.
///
/// Behaviour mirrors [`axum::Json`]:
/// - As an extractor: requires `Content-Type: application/json` (or `*/*+json`),
///   buffers the body, and deserializes into `T`.
/// - As a response: serializes `T` to JSON and sets `Content-Type: application/json`.
///
/// Extraction failures become [`ClaudeError`] (HTTP 400 + Anthropic error JSON).
#[derive(Debug, Clone, Copy, Default)]
#[must_use]
pub struct ClaudeJson<T>(pub T);

impl<T> ClaudeJson<T> {
  /// Unwrap the inner value.
  pub fn into_inner(self) -> T {
    self.0
  }
}

impl<T> From<T> for ClaudeJson<T> {
  fn from(inner: T) -> Self {
    Self(inner)
  }
}

impl<T> std::ops::Deref for ClaudeJson<T> {
  type Target = T;

  fn deref(&self) -> &Self::Target {
    &self.0
  }
}

impl<T> std::ops::DerefMut for ClaudeJson<T> {
  fn deref_mut(&mut self) -> &mut Self::Target {
    &mut self.0
  }
}

impl<T, S> FromRequest<S> for ClaudeJson<T>
where
  T: DeserializeOwned,
  S: Send + Sync,
{
  type Rejection = ClaudeError;

  async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
    if !json_content_type(req.headers()) {
      return Err(ClaudeError::bad_request(
        "Content-Type must be 'application/json'",
      ));
    }

    let bytes = Bytes::from_request(req, state).await.map_err(|err| {
      ClaudeError::bad_request(format!("Failed to buffer the request body: {err}"))
    })?;

    Self::from_bytes(&bytes)
  }
}

impl<T> ClaudeJson<T>
where
  T: DeserializeOwned,
{
  /// Deserialize `T` from a raw JSON byte slice.
  pub fn from_bytes(bytes: &[u8]) -> Result<Self, ClaudeError> {
    if bytes.is_empty() {
      return Err(ClaudeError::bad_request("Request body is empty"));
    }

    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let value = T::deserialize(&mut deserializer).map_err(|err| {
      ClaudeError::bad_request(format!("Failed to parse the request body as JSON: {err}"))
    })?;

    deserializer.end().map_err(|err| {
      ClaudeError::bad_request(format!("Failed to parse the request body as JSON: {err}"))
    })?;

    Ok(Self(value))
  }
}

impl<T> IntoResponse for ClaudeJson<T>
where
  T: Serialize,
{
  fn into_response(self) -> Response {
    fn make_response(buf: BytesMut, ser_result: serde_json::Result<()>) -> Response {
      match ser_result {
        Ok(()) => (
          [(
            header::CONTENT_TYPE,
            HeaderValue::from_static(mime::APPLICATION_JSON.as_ref()),
          )],
          buf.freeze(),
        )
          .into_response(),
        Err(err) => ClaudeError(AppError::Internal(format!(
          "Failed to serialize response as JSON: {err}"
        )))
        .into_response(),
      }
    }

    let mut buf = BytesMut::with_capacity(128).writer();
    let res = serde_json::to_writer(&mut buf, &self.0);
    make_response(buf.into_inner(), res)
  }
}

fn json_content_type(headers: &HeaderMap) -> bool {
  let Some(content_type) = headers.get(header::CONTENT_TYPE) else {
    return false;
  };
  let Ok(content_type) = content_type.to_str() else {
    return false;
  };
  let Ok(mime) = content_type.parse::<mime::Mime>() else {
    return false;
  };

  mime.type_() == "application"
    && (mime.subtype() == "json" || mime.suffix().is_some_and(|name| name == "json"))
}

#[cfg(test)]
mod tests {
  use super::*;
  use axum::Router;
  use axum::body::Body;
  use axum::http::{Request, StatusCode};
  use axum::routing::post;
  use serde::Deserialize;
  use tower::ServiceExt;

  #[derive(Debug, Deserialize)]
  struct Input {
    foo: String,
  }

  async fn echo(ClaudeJson(input): ClaudeJson<Input>) -> String {
    input.foo
  }

  fn app() -> Router {
    Router::new().route("/", post(echo))
  }

  #[tokio::test]
  async fn deserializes_json_body() {
    let res = app()
      .oneshot(
        Request::builder()
          .method("POST")
          .uri("/")
          .header("content-type", "application/json")
          .body(Body::from(r#"{"foo":"bar"}"#))
          .unwrap(),
      )
      .await
      .unwrap();

    assert_eq!(res.status(), StatusCode::OK);
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
      .await
      .unwrap();
    assert_eq!(&body[..], b"bar");
  }

  #[tokio::test]
  async fn rejects_missing_content_type_with_anthropic_error() {
    let res = app()
      .oneshot(
        Request::builder()
          .method("POST")
          .uri("/")
          .body(Body::from(r#"{"foo":"bar"}"#))
          .unwrap(),
      )
      .await
      .unwrap();

    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
      .await
      .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["type"], "error");
    assert_eq!(v["error"]["type"], "invalid_request_error");
    assert!(
      v["error"]["message"]
        .as_str()
        .unwrap()
        .contains("Content-Type")
    );
  }

  #[tokio::test]
  async fn rejects_invalid_json_with_anthropic_error() {
    let res = app()
      .oneshot(
        Request::builder()
          .method("POST")
          .uri("/")
          .header("content-type", "application/json")
          .body(Body::from("{"))
          .unwrap(),
      )
      .await
      .unwrap();

    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
      .await
      .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["type"], "error");
    assert_eq!(v["error"]["type"], "invalid_request_error");
    assert!(
      v["error"]["message"]
        .as_str()
        .unwrap()
        .contains("Failed to parse")
    );
  }

  #[tokio::test]
  async fn rejects_type_mismatch_with_anthropic_error() {
    let res = app()
      .oneshot(
        Request::builder()
          .method("POST")
          .uri("/")
          .header("content-type", "application/json")
          .body(Body::from(r#"{"foo":1}"#))
          .unwrap(),
      )
      .await
      .unwrap();

    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
      .await
      .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["type"], "error");
    assert_eq!(v["error"]["type"], "invalid_request_error");
  }
}
