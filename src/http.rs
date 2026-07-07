use crate::adapters::parse_adapter_bytes;
use crate::models::{AdapterKind, ConflictKind, IndexPayload};
use crate::resolution::build_index;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

pub type SharedState = Arc<RwLock<IndexPayload>>;

pub fn router(payload: IndexPayload) -> Router {
    let state = Arc::new(RwLock::new(payload));
    Router::new()
        .route("/healthz", get(healthz))
        .route("/ingest/{adapter}", post(ingest))
        .route("/dossier/{parcel_id}", get(dossier))
        .route("/conflicts", get(conflicts))
        .route("/coverage", get(coverage))
        .with_state(state)
}

async fn healthz() -> &'static str {
    "ok"
}

async fn ingest(
    Path(adapter): Path<String>,
    State(state): State<SharedState>,
    body: Bytes,
) -> Result<impl IntoResponse, ApiError> {
    let adapter: AdapterKind = adapter.parse().map_err(ApiError::bad_request)?;
    let records = parse_adapter_bytes(adapter, &body)
        .map_err(|err| ApiError::bad_request(err.to_string()))?;
    let payload = build_index(records);
    let coverage = payload.coverage.clone();
    *state.write().await = payload;
    Ok((StatusCode::CREATED, Json(coverage)))
}

async fn dossier(
    Path(parcel_id): Path<String>,
    State(state): State<SharedState>,
) -> Result<impl IntoResponse, ApiError> {
    let payload = state.read().await;
    let requested = parcel_id.to_ascii_uppercase();
    let dossier = payload
        .parcels
        .iter()
        .map(|parcel| &parcel.dossier)
        .find(|dossier| {
            dossier.parcel_id.eq_ignore_ascii_case(&parcel_id)
                || dossier
                    .parcel_id
                    .replace('-', "")
                    .eq_ignore_ascii_case(&requested)
        })
        .ok_or_else(|| ApiError::not_found("dossier not found"))?;
    Ok(Json(dossier.clone()))
}

async fn conflicts(
    Query(query): Query<HashMap<String, String>>,
    State(state): State<SharedState>,
) -> Result<impl IntoResponse, ApiError> {
    let kind = query
        .get("kind")
        .map(|value| value.parse::<ConflictKind>())
        .transpose()
        .map_err(ApiError::bad_request)?;
    let payload = state.read().await;
    let conflicts: Vec<_> = payload
        .conflicts
        .iter()
        .filter(|conflict| kind.is_none_or(|kind| conflict.kind == kind))
        .cloned()
        .collect();
    Ok(Json(conflicts))
}

async fn coverage(State(state): State<SharedState>) -> impl IntoResponse {
    let payload = state.read().await;
    Json(payload.coverage.clone())
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn bad_request(message: impl ToString) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.to_string(),
        }
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: message.into(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, self.message).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    #[tokio::test]
    async fn http_endpoints_return_seeded_index() {
        let csv = "parcel_id,address,owner,gclba_owned,property_class\n\
                   40-12-345-067,123 North Saginaw Street,GCLBA,true,Residential";
        let app = router(IndexPayload::default());
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/ingest/site_control_export")
                    .body(Body::from(csv))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/coverage")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
}
