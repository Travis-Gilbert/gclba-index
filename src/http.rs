use crate::export_store::ExportStore;
use crate::models::{AdapterKind, ConflictKind, IndexPayload};
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Clone, Debug)]
pub struct AppState {
    payload: Arc<RwLock<IndexPayload>>,
    store: ExportStore,
}

pub fn router(payload: IndexPayload) -> Router {
    router_with_store(payload, ExportStore::new("exports"))
}

pub fn router_with_store(payload: IndexPayload, store: ExportStore) -> Router {
    let state = AppState {
        payload: Arc::new(RwLock::new(payload)),
        store,
    };
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
    State(state): State<AppState>,
    body: Bytes,
) -> Result<impl IntoResponse, ApiError> {
    let adapter: AdapterKind = adapter.parse().map_err(ApiError::bad_request)?;
    state
        .store
        .save_export(adapter, &body)
        .map_err(|err| ApiError::bad_request(err.to_string()))?;
    let payload = state
        .store
        .rebuild_index()
        .map_err(|err| ApiError::bad_request(err.to_string()))?;
    let coverage = payload.coverage.clone();
    *state.payload.write().await = payload;
    Ok((StatusCode::CREATED, Json(coverage)))
}

async fn dossier(
    Path(parcel_id): Path<String>,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ApiError> {
    let payload = state.payload.read().await;
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
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ApiError> {
    let kind = query
        .get("kind")
        .map(|value| value.parse::<ConflictKind>())
        .transpose()
        .map_err(ApiError::bad_request)?;
    let payload = state.payload.read().await;
    let conflicts: Vec<_> = payload
        .conflicts
        .iter()
        .filter(|conflict| kind.is_none_or(|kind| conflict.kind == kind))
        .cloned()
        .collect();
    Ok(Json(conflicts))
}

async fn coverage(State(state): State<AppState>) -> impl IntoResponse {
    let payload = state.payload.read().await;
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
    use axum::body::to_bytes;
    use axum::body::Body;
    use axum::http::Request;
    use serde_json::Value;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};
    use tower::ServiceExt;

    #[tokio::test]
    async fn http_ingest_persists_and_rebuilds_combined_index() {
        let root = test_dir("http-ingest");
        let csv = "parcel_id,address,owner,gclba_owned,property_class\n\
                   40-12-345-067,123 North Saginaw Street,GCLBA,true,Residential";
        let filemaker = "parcel_id,address,program\n\
                         40-12-345-067,123 North Saginaw Street,Homebuyer";
        let app = router_with_store(IndexPayload::default(), ExportStore::new(&root));
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
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/ingest/filemaker_csv")
                    .body(Body::from(filemaker))
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
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let coverage: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(coverage["parcelsIndexed"], 1);
        assert_eq!(coverage["sourceCount"], 2);

        assert!(root.join("site_control_export").exists());
        assert!(root.join("filemaker_csv").exists());
        fs::remove_dir_all(root).ok();
    }

    fn test_dir(name: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "gclba-index-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        path
    }
}
