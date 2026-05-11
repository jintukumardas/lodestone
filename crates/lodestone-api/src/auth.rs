//! Bearer-token middleware.
//!
//! Every route except `/healthz` requires `Authorization: Bearer <token>`
//! where `<token>` matches `LODESTONE_API_TOKEN`. The comparison is
//! constant-time so we don't leak the secret via timing.

use axum::{
    body::Body,
    extract::State,
    http::{header, Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use subtle::ConstantTimeEq;

#[derive(Clone)]
pub struct ApiToken(pub String);

pub async fn require_bearer(
    State(token): State<ApiToken>,
    req: Request<Body>,
    next: Next,
) -> Response {
    if req.uri().path() == "/healthz" {
        return next.run(req).await;
    }

    let presented = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "));

    match presented {
        Some(p) if p.as_bytes().ct_eq(token.0.as_bytes()).into() => next.run(req).await,
        Some(_) => unauthorized("bad token"),
        None => unauthorized("missing bearer token"),
    }
}

fn unauthorized(msg: &str) -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(serde_json::json!({ "error": msg })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{routing::get, Router};
    use tower::ServiceExt;

    fn test_app(token: &str) -> Router {
        Router::new()
            .route("/healthz", get(|| async { "ok" }))
            .route("/protected", get(|| async { "secret" }))
            .layer(axum::middleware::from_fn_with_state(
                ApiToken(token.into()),
                require_bearer,
            ))
    }

    async fn status(app: Router, uri: &str, auth: Option<&str>) -> StatusCode {
        let mut req = Request::builder().uri(uri);
        if let Some(a) = auth {
            req = req.header(header::AUTHORIZATION, a);
        }
        app.oneshot(req.body(Body::empty()).unwrap())
            .await
            .unwrap()
            .status()
    }

    #[tokio::test]
    async fn healthz_is_public() {
        assert_eq!(status(test_app("secret"), "/healthz", None).await, StatusCode::OK);
    }

    #[tokio::test]
    async fn missing_token_rejected() {
        assert_eq!(
            status(test_app("secret"), "/protected", None).await,
            StatusCode::UNAUTHORIZED
        );
    }

    #[tokio::test]
    async fn wrong_token_rejected() {
        assert_eq!(
            status(test_app("secret"), "/protected", Some("Bearer nope")).await,
            StatusCode::UNAUTHORIZED
        );
    }

    #[tokio::test]
    async fn malformed_header_rejected() {
        assert_eq!(
            status(test_app("secret"), "/protected", Some("secret")).await,
            StatusCode::UNAUTHORIZED
        );
    }

    #[tokio::test]
    async fn correct_token_accepted() {
        assert_eq!(
            status(test_app("secret"), "/protected", Some("Bearer secret")).await,
            StatusCode::OK
        );
    }
}
