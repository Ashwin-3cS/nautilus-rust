use crate::AppState;
use crate::EnclaveError;
use axum::extract::{Query, State};
use axum::Json;
use nautilus_enclave::EnclaveKeyPair;
use serde::{Deserialize, Serialize};
use serde_repr::{Deserialize_repr, Serialize_repr};
use std::sync::Arc;
use tracing::info;

// ── Intent message types (matches Sui on-chain IntentMessage<T>) ──────

#[derive(Debug, Serialize, Deserialize)]
pub struct IntentMessage<T: Serialize> {
    pub intent: IntentScope,
    pub timestamp_ms: u64,
    pub data: T,
}

#[derive(Serialize_repr, Deserialize_repr, Debug)]
#[repr(u8)]
pub enum IntentScope {
    Generic = 0,
    SignName = 1,
}

impl<T: Serialize + std::fmt::Debug> IntentMessage<T> {
    pub fn new(data: T, timestamp_ms: u64, intent: IntentScope) -> Self {
        Self { data, timestamp_ms, intent }
    }
}

#[derive(Serialize, Deserialize)]
pub struct ProcessedDataResponse<T> {
    pub response: T,
    pub signature: String,
}

/// Sign the BCS bytes of the payload with the enclave keypair.
pub fn to_signed_response<T: Serialize + Clone>(
    kp: &EnclaveKeyPair,
    payload: T,
    timestamp_ms: u64,
    intent: IntentScope,
) -> ProcessedDataResponse<IntentMessage<T>> {
    let intent_msg = IntentMessage {
        intent,
        timestamp_ms,
        data: payload.clone(),
    };

    let signing_payload = bcs::to_bytes(&intent_msg).expect("should not fail");
    let sig = kp.sign(&signing_payload);
    ProcessedDataResponse {
        response: intent_msg,
        signature: hex::encode(sig.to_bytes()),
    }
}

// ── GET /get_attestation ──────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub struct GetAttestationResponse {
    pub attestation: String,
}

pub async fn get_attestation(
    State(state): State<Arc<AppState>>,
) -> Result<Json<GetAttestationResponse>, EnclaveError> {
    info!("get_attestation called");

    let pk_bytes = state.eph_kp.public_key_bytes();
    let doc = nautilus_enclave::get_attestation(&pk_bytes, &[])
        .map_err(|e| EnclaveError::GenericError(format!("attestation failed: {}", e)))?;

    Ok(Json(GetAttestationResponse {
        attestation: doc.raw_cbor_hex,
    }))
}

// ── GET /health ───────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub struct HealthCheckResponse {
    pub public_key: String,
    pub status: String,
}

pub async fn health_check(
    State(state): State<Arc<AppState>>,
) -> Result<Json<HealthCheckResponse>, EnclaveError> {
    let pk = state.eph_kp.public_key_bytes();
    Ok(Json(HealthCheckResponse {
        public_key: hex::encode(pk),
        status: "ok".to_string(),
    }))
}

// ── POST /sign_name ──────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SignNameRequest {
    pub name: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SignedName {
    pub name: String,
    pub message: String,
}

pub async fn sign_name(
    State(state): State<Arc<AppState>>,
    Json(req): Json<SignNameRequest>,
) -> Result<Json<ProcessedDataResponse<IntentMessage<SignedName>>>, EnclaveError> {
    info!("sign_name called for: {}", req.name);

    let timestamp_ms = chrono::Utc::now().timestamp_millis() as u64;

    let signed_name = SignedName {
        name: req.name.clone(),
        message: format!("Hello {}! This message was signed inside a Nitro Enclave.", req.name),
    };

    let response = to_signed_response(
        &state.eph_kp,
        signed_name,
        timestamp_ms,
        IntentScope::SignName,
    );

    Ok(Json(response))
}

// ── GET /logs ────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct LogsQueryParams {
    pub lines: Option<usize>,
}

#[derive(Serialize)]
pub struct LogsResponse {
    pub lines: Vec<String>,
    pub count: usize,
}

pub async fn get_logs(
    State(state): State<Arc<AppState>>,
    Query(params): Query<LogsQueryParams>,
) -> Result<Json<LogsResponse>, EnclaveError> {
    let n = params.lines.unwrap_or(100).min(1000);
    let lines = state.logs.recent(n);
    Ok(Json(LogsResponse {
        count: lines.len(),
        lines,
    }))
}
