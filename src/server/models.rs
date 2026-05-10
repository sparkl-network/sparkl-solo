use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde_json::{json, Value};

use super::AppState;

pub async fn list_models(State(state): State<AppState>) -> impl IntoResponse {
    match state.proxy.list_models().await {
        Ok(models) => {
            let data = models
                .into_iter()
                .filter(|m| super::is_model_allowed(&state, &m.id))
                .map(|m| json!({ "id": m.id }))
                .collect::<Vec<Value>>();
            (
                StatusCode::OK,
                Json(json!({ "object": "list", "data": data })),
            )
                .into_response()
        }
        Err(err) => (
            StatusCode::BAD_GATEWAY,
            Json(json!({ "error": format!("backend models unavailable: {err}") })),
        )
            .into_response(),
    }
}
