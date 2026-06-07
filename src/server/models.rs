use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde_json::json;

use crate::models::{build_catalog, catalog_to_openai_list};

use super::AppState;

pub async fn list_models(State(state): State<AppState>) -> impl IntoResponse {
    match build_catalog(
        &state.proxy,
        &state.config.models,
        &state.config.node,
        &state.admission,
    )
    .await
    {
        Ok(models) => (StatusCode::OK, Json(catalog_to_openai_list(&models))).into_response(),
        Err(err) => (
            StatusCode::BAD_GATEWAY,
            Json(json!({ "error": format!("backend models unavailable: {err}") })),
        )
            .into_response(),
    }
}
