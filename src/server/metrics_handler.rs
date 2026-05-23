use axum::http::StatusCode;
use axum::response::IntoResponse;

/// Prometheus metrics endpoint (/metrics).
pub async fn prometheus_metrics() -> impl IntoResponse {
    let body = crate::metrics::encode_prometheus();
    if body.is_empty() {
        (StatusCode::SERVICE_UNAVAILABLE, "no metrics registered").into_response()
    } else {
        (
            StatusCode::OK,
            [("content-type", "text/plain; version=0.0.4")],
            body,
        )
            .into_response()
    }
}
