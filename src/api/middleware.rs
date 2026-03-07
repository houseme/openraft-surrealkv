//! HTTP middleware utilities and common extension points for the HTTP API.
//!
//! Provides a small set of reusable middleware helpers that the HTTP server can opt-in to.
//!
//! Included helpers:
//! - `request_id_middleware`: generates/propagates `x-request-id` and attaches it to request extensions
//! - `timeout_layer`: convenience constructor for `tower::timeout::TimeoutLayer`
//! - `rate_limit_layer`: convenience constructor for `tower::limit::RateLimitLayer`
//! - `default_service_builder`: a small composition of commonly used layers (trace left to caller)

use axum::extract::Request;
use axum::http::header;
use axum::middleware::Next;
use axum::response::Response;
use std::time::Duration;
use tower::limit::RateLimitLayer;
use tower::timeout::TimeoutLayer;
use tower::ServiceBuilder;
use uuid::Uuid;

/// Ensure every request has an `x-request-id` header.
///
/// - If the incoming request already contains `x-request-id`, the same value is used.
/// - Otherwise a new UUIDv4 is generated and added to the request headers and response headers.
///
/// The generated request-id is also stored in request extensions under the header name, so
/// handlers and other middleware can retrieve it via `req.extensions()`.
pub async fn request_id_middleware(mut req: Request, next: Next) -> Response {
    // Check existing header
    let req_id = req
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    // Insert into request extensions for downstream access
    req.extensions_mut().insert(req_id.clone());

    // Continue processing
    let mut resp = next.run(req).await;

    // Ensure response contains the request-id header for correlation
    if resp.headers().get("x-request-id").is_none()
        && let Ok(value) = header::HeaderValue::from_str(&req_id)
    {
        resp.headers_mut()
            .insert(header::HeaderName::from_static("x-request-id"), value);
    }

    resp
}

/// Create a `TimeoutLayer` with the provided duration.
pub fn timeout_layer(dur: Duration) -> TimeoutLayer {
    TimeoutLayer::new(dur)
}

/// Create a `RateLimitLayer` permitting `max_requests` requests per `per` duration.
///
/// Note: This is a simple per-service rate limiter using `tower::limit::RateLimitLayer`.
pub fn rate_limit_layer(max_requests: u64, per: Duration) -> RateLimitLayer {
    RateLimitLayer::new(max_requests, per)
}

/// Build a convenience `ServiceBuilder` with common middleware layers.
///
/// This builder intentionally does not include tracing (TraceLayer) because the server
/// already wires a `TraceLayer` with header inclusion; include it explicitly when needed.
pub type DefaultServiceBuilder =
    ServiceBuilder<tower::layer::util::Stack<TimeoutLayer, tower::layer::util::Identity>>;

pub fn default_service_builder(node_id: impl Into<String>) -> DefaultServiceBuilder {
    let _node = node_id.into();
    ServiceBuilder::new()
        // Keep a short default timeout to protect handlers from lingering
        .layer(TimeoutLayer::new(Duration::from_secs(15)))
}
