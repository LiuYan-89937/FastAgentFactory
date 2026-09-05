use reqwest::header::{ACCEPT, USER_AGENT};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::time::Duration;
use tauri::AppHandle;

use crate::AppState;

const COMBO_SERVICE_URL: &str = env!("COMBO_SERVICE_URL");
const REPORT_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorReportInput {
    summary: String,
    error_code: Option<String>,
    request_id: Option<String>,
    diagnostic_ref: Option<String>,
    context: Option<Value>,
}

#[derive(Serialize)]
struct ErrorReportPayload {
    source: &'static str,
    app_version: String,
    platform: &'static str,
    architecture: &'static str,
    error_code: String,
    summary: String,
    request_id: String,
    diagnostic_ref: String,
    context: Value,
    log_excerpt: String,
}

#[derive(Deserialize, Serialize)]
pub struct ErrorReportReceipt {
    error_report_id: String,
    status: String,
    created_at: String,
}

#[derive(Deserialize)]
struct ServiceErrorEnvelope {
    error: Option<ServiceError>,
    detail: Option<Value>,
}

#[derive(Deserialize)]
struct ServiceError {
    message: String,
}

#[tauri::command]
pub async fn report_error(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    input: ErrorReportInput,
) -> Result<ErrorReportReceipt, String> {
    let summary = input.summary.trim();
    if summary.is_empty() {
        return Err("Error summary must not be empty".to_string());
    }

    let log_excerpt = {
        let sidecar = state
            .sidecar
            .lock()
            .map_err(|_| "Backend state is unavailable".to_string())?;
        sidecar
            .as_ref()
            .and_then(|running| running.log_tail())
            .map(|value| redact_text(&value))
            .unwrap_or_default()
    };

    let payload = ErrorReportPayload {
        source: "desktop",
        app_version: app.package_info().version.to_string(),
        platform: std::env::consts::OS,
        architecture: std::env::consts::ARCH,
        error_code: truncate_chars(&clean_optional(input.error_code), 200),
        summary: truncate_chars(&redact_text(summary), 4_000),
        request_id: truncate_chars(&clean_optional(input.request_id), 200),
        diagnostic_ref: truncate_chars(&clean_optional(input.diagnostic_ref), 500),
        context: redact_json(input.context.unwrap_or_else(|| Value::Object(Map::new()))),
        log_excerpt,
    };

    let response = reqwest::Client::builder()
        .timeout(REPORT_TIMEOUT)
        .build()
        .map_err(error_text)?
        .post(service_endpoint("/api/v1/error-reports"))
        .header(USER_AGENT, "Combo-Desktop")
        .header(ACCEPT, "application/json")
        .json(&payload)
        .send()
        .await
        .map_err(error_text)?;

    if !response.status().is_success() {
        let status = response.status();
        let fallback = format!("Error report upload failed with HTTP {status}");
        let body = response.json::<ServiceErrorEnvelope>().await.ok();
        return Err(body
            .and_then(|value| {
                value
                    .error
                    .map(|error| error.message)
                    .or_else(|| value.detail.map(|detail| detail.to_string()))
            })
            .filter(|message| !message.trim().is_empty())
            .unwrap_or(fallback));
    }

    response
        .json::<ErrorReportReceipt>()
        .await
        .map_err(error_text)
}

fn clean_optional(value: Option<String>) -> String {
    value.unwrap_or_default().trim().to_string()
}

fn truncate_chars(value: &str, maximum: usize) -> String {
    value.chars().take(maximum).collect()
}

fn service_endpoint(path: &str) -> String {
    format!("{}{}", COMBO_SERVICE_URL.trim_end_matches('/'), path)
}

fn error_text(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn redact_json(value: Value) -> Value {
    match value {
        Value::Object(values) => Value::Object(
            values
                .into_iter()
                .map(|(key, value)| {
                    let sanitized = if sensitive_key(&key) {
                        Value::String("[REDACTED]".to_string())
                    } else {
                        redact_json(value)
                    };
                    (key, sanitized)
                })
                .collect(),
        ),
        Value::Array(values) => Value::Array(values.into_iter().map(redact_json).collect()),
        Value::String(value) => Value::String(redact_text(&value)),
        other => other,
    }
}

fn sensitive_key(key: &str) -> bool {
    let normalized = key.to_ascii_lowercase().replace(['-', ' '], "_");
    [
        "authorization",
        "api_key",
        "apikey",
        "access_token",
        "refresh_token",
        "password",
        "secret",
        "credential",
    ]
    .iter()
    .any(|marker| normalized.contains(marker))
}

fn redact_text(value: &str) -> String {
    value
        .lines()
        .map(|line| {
            let normalized = line.to_ascii_lowercase();
            if [
                "authorization",
                "bearer ",
                "api_key",
                "api-key",
                "apikey",
                "access_token",
                "refresh_token",
                "password",
                "secret",
                "token=",
                "key=",
                "sk-",
            ]
            .iter()
            .any(|marker| normalized.contains(marker))
            {
                "[REDACTED SENSITIVE LINE]"
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}
