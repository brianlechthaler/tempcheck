use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};
use rmcp::{
    handler::server::{tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, Content, ErrorData as McpError, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router, ServerHandler, ServiceExt,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::audit::{AuditEvent, AuditSink, FileAuditSink, MultiAuditSink, StderrAuditSink};
use crate::collector::{SysfsCollector, TemperatureCollector};
use crate::config::{DEFAULT_MCP_TOKEN_ENV, MAX_RESPONSE_BYTES};
use crate::storage::{SqliteStore, TemperatureStore};

pub const TOOL_GET_CURRENT: &str = "get_current_temperature";
pub const TOOL_GET_CURRENT_VERSION: &str = "1";
pub const TOOL_ANALYZE: &str = "analyze_temperature";
pub const TOOL_ANALYZE_VERSION: &str = "1";

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AnalyzeTemperatureParams {
    /// ISO-8601 start time (inclusive). Defaults to 24 hours ago.
    #[serde(default)]
    pub from: Option<String>,
    /// ISO-8601 end time (inclusive). Defaults to now.
    #[serde(default)]
    pub to: Option<String>,
    /// Filter to a specific sensor name (e.g. thermal_zone0:cpu).
    #[serde(default)]
    pub sensor: Option<String>,
    /// Bearer token when TEMPCHECK_MCP_TOKEN is set on the server.
    #[serde(default)]
    pub auth_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GetCurrentTemperatureParams {
    /// Bearer token when TEMPCHECK_MCP_TOKEN is set on the server.
    #[serde(default)]
    pub auth_token: Option<String>,
}

#[derive(Clone)]
pub struct TemperatureMcpService {
    store: Arc<SqliteStore>,
    collector: Arc<dyn TemperatureCollector>,
    audit: Arc<dyn AuditSink>,
    expected_token: Option<String>,
    tool_router: ToolRouter<Self>,
}

impl TemperatureMcpService {
    pub fn from_parts(
        store: Arc<SqliteStore>,
        collector: Arc<dyn TemperatureCollector>,
        audit: Arc<dyn AuditSink>,
        expected_token: Option<String>,
    ) -> Self {
        Self {
            store,
            collector,
            audit,
            expected_token,
            tool_router: Self::tool_router(),
        }
    }

    pub fn new(db_path: impl AsRef<Path>, audit_log: Option<std::path::PathBuf>) -> Result<Self> {
        let store =
            Arc::new(SqliteStore::open(db_path).context("failed to open temperature database")?);
        let collector: Arc<dyn TemperatureCollector> = Arc::new(SysfsCollector::new());
        let expected_token = std::env::var(DEFAULT_MCP_TOKEN_ENV).ok();

        let mut sinks: Vec<Arc<dyn AuditSink>> = vec![Arc::new(StderrAuditSink)];
        if let Some(path) = audit_log {
            let file_sink = FileAuditSink::open(path).context("failed to open audit log")?;
            sinks.push(Arc::new(file_sink));
        }
        let audit: Arc<dyn AuditSink> = Arc::new(MultiAuditSink::new(sinks));

        Ok(Self::from_parts(store, collector, audit, expected_token))
    }

    fn authorize(&self, token: Option<&str>) -> Result<(), McpError> {
        match &self.expected_token {
            None => Ok(()),
            Some(expected) => match token {
                Some(t) if t == expected => Ok(()),
                _ => Err(McpError::invalid_params(
                    "insufficient scope: valid auth_token required",
                    None,
                )),
            },
        }
    }

    fn audit_invoke(&self, tool: &str, version: &str, params: &str) -> AuditEvent {
        let correlation_id = Uuid::new_v4().to_string();
        let event = AuditEvent::invoke(
            tool,
            version,
            "mcp-client",
            "temperature:read",
            params,
            &correlation_id,
        );
        self.audit.emit(&event);
        event
    }

    fn audit_result(&self, event: AuditEvent, status: &str, bytes: usize, err: Option<String>) {
        self.audit.emit(&event.with_result(status, bytes, err));
    }

    fn cap_response(json: &str) -> Result<String, McpError> {
        if json.len() > MAX_RESPONSE_BYTES {
            return Err(McpError::internal_error(
                format!("response exceeds {MAX_RESPONSE_BYTES} byte cap"),
                None,
            ));
        }
        Ok(json.to_string())
    }

    fn parse_time(s: &str) -> Result<DateTime<Utc>, McpError> {
        DateTime::parse_from_rfc3339(s)
            .map(|dt| dt.with_timezone(&Utc))
            .map_err(|_| McpError::invalid_params("invalid ISO-8601 timestamp", None))
    }
}

#[tool_router]
impl TemperatureMcpService {
    #[tool(
        name = "get_current_temperature",
        description = "get_current_temperature@1 — Read live system thermal sensor readings from /sys/class/thermal."
    )]
    async fn get_current_temperature(
        &self,
        params: Parameters<GetCurrentTemperatureParams>,
    ) -> Result<CallToolResult, McpError> {
        let params_json = serde_json::to_string(&params.0).unwrap_or_else(|_| "{}".to_string());
        let event = self.audit_invoke(TOOL_GET_CURRENT, TOOL_GET_CURRENT_VERSION, &params_json);

        let result: Result<CallToolResult, McpError> = async {
            self.authorize(params.0.auth_token.as_deref())?;
            let readings = self
                .collector
                .collect()
                .map_err(|e| McpError::internal_error(e.to_string(), None))?;

            let payload = serde_json::to_string(&readings)
                .map_err(|e| McpError::internal_error(e.to_string(), None))?;
            let payload = Self::cap_response(&payload)?;
            self.audit_result(event.clone(), "success", payload.len(), None);
            Ok(CallToolResult::success(vec![Content::text(payload)]))
        }
        .await;

        if let Err(ref e) = result {
            self.audit_result(event, "error", 0, Some(e.to_string()));
        }
        result
    }

    #[tool(
        name = "analyze_temperature",
        description = "analyze_temperature@1 — Query stored temperature history with min/max/avg stats over a time range."
    )]
    async fn analyze_temperature(
        &self,
        params: Parameters<AnalyzeTemperatureParams>,
    ) -> Result<CallToolResult, McpError> {
        let params_json = serde_json::to_string(&params.0).unwrap_or_else(|_| "{}".to_string());
        let event = self.audit_invoke(TOOL_ANALYZE, TOOL_ANALYZE_VERSION, &params_json);

        let result: Result<CallToolResult, McpError> = async {
            self.authorize(params.0.auth_token.as_deref())?;

            let to = match &params.0.to {
                Some(s) => Self::parse_time(s)?,
                None => Utc::now(),
            };
            let from = match &params.0.from {
                Some(s) => Self::parse_time(s)?,
                None => to - Duration::hours(24),
            };

            if from >= to {
                return Err(McpError::invalid_params("from must be before to", None));
            }

            let sensor = params.0.sensor.as_deref();
            let analysis = self.store.analyze(sensor, from, to).map_err(|e| {
                let msg = e.to_string();
                if msg.contains("invalid time range") || msg.contains("row limit") {
                    McpError::invalid_params(msg, None)
                } else {
                    McpError::internal_error(msg, None)
                }
            })?;

            let payload = serde_json::to_string(&analysis)
                .map_err(|e| McpError::internal_error(e.to_string(), None))?;
            let payload = Self::cap_response(&payload)?;
            self.audit_result(event.clone(), "success", payload.len(), None);
            Ok(CallToolResult::success(vec![Content::text(payload)]))
        }
        .await;

        if let Err(ref e) = result {
            self.audit_result(event, "error", 0, Some(e.to_string()));
        }
        result
    }
}

#[tool_handler]
impl ServerHandler for TemperatureMcpService {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(
                "Temperature monitoring MCP server. Tools are version-pinned: \
                 get_current_temperature@1 reads live sensors; analyze_temperature@1 \
                 queries historical SQLite data. Set TEMPCHECK_MCP_TOKEN and pass \
                 auth_token when auth is enabled."
                    .into(),
            ),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}

pub async fn run_mcp_server(
    db_path: impl AsRef<Path>,
    audit_log: Option<std::path::PathBuf>,
) -> Result<()> {
    let service = TemperatureMcpService::new(db_path, audit_log)?;
    let running = service
        .serve(rmcp::transport::stdio())
        .await
        .context("failed to start MCP server")?;
    running
        .waiting()
        .await
        .context("MCP server exited with error")
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collector::{MockCollector, SensorReading};

    fn test_service(
        store: Arc<SqliteStore>,
        collector: Arc<dyn TemperatureCollector>,
        token: Option<String>,
    ) -> TemperatureMcpService {
        TemperatureMcpService::from_parts(store, collector, Arc::new(StderrAuditSink), token)
    }

    #[test]
    fn authorize_allows_when_no_token_configured() {
        let store = Arc::new(SqliteStore::in_memory().unwrap());
        let svc = test_service(store, Arc::new(SysfsCollector::new()), None);
        assert!(svc.authorize(None).is_ok());
    }

    #[test]
    fn authorize_rejects_missing_token() {
        let store = Arc::new(SqliteStore::in_memory().unwrap());
        let svc = test_service(
            store,
            Arc::new(SysfsCollector::new()),
            Some("secret".to_string()),
        );
        assert!(svc.authorize(None).is_err());
        assert!(svc.authorize(Some("wrong")).is_err());
        assert!(svc.authorize(Some("secret")).is_ok());
    }

    #[test]
    fn cap_response_rejects_oversized_payload() {
        let big = "x".repeat(MAX_RESPONSE_BYTES + 1);
        assert!(TemperatureMcpService::cap_response(&big).is_err());
    }

    #[test]
    fn parse_time_accepts_rfc3339() {
        let dt = TemperatureMcpService::parse_time("2025-01-01T00:00:00Z").unwrap();
        assert_eq!(dt.to_rfc3339(), "2025-01-01T00:00:00+00:00");
    }

    #[test]
    fn parse_time_rejects_invalid_input() {
        assert!(TemperatureMcpService::parse_time("not-a-date").is_err());
    }

    #[tokio::test]
    async fn analyze_temperature_rejects_bad_timestamp() {
        let store = Arc::new(SqliteStore::in_memory().unwrap());
        let svc = test_service(store, Arc::new(MockCollector::new(vec![])), None);

        let err = svc
            .analyze_temperature(Parameters(AnalyzeTemperatureParams {
                from: Some("invalid".into()),
                to: None,
                sensor: None,
                auth_token: None,
            }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("ISO-8601"));
    }

    #[tokio::test]
    async fn analyze_temperature_uses_default_time_range() {
        let store = Arc::new(SqliteStore::in_memory().unwrap());
        let svc = test_service(store, Arc::new(MockCollector::new(vec![])), None);

        let result = svc
            .analyze_temperature(Parameters(AnalyzeTemperatureParams {
                from: None,
                to: None,
                sensor: None,
                auth_token: None,
            }))
            .await
            .unwrap();

        let text = result.content[0].as_text().unwrap();
        assert!(text.text.contains("[]") || text.text.contains("sensor_name"));
    }

    #[tokio::test]
    async fn analyze_temperature_denies_without_token() {
        let store = Arc::new(SqliteStore::in_memory().unwrap());
        let svc = test_service(
            store,
            Arc::new(MockCollector::new(vec![])),
            Some("tok".into()),
        );
        assert!(svc
            .analyze_temperature(Parameters(AnalyzeTemperatureParams {
                from: None,
                to: None,
                sensor: None,
                auth_token: None,
            }))
            .await
            .is_err());
    }

    #[tokio::test]
    async fn get_current_temperature_tool_returns_mock_readings() {
        let reading = SensorReading {
            sensor_name: "thermal_zone0:cpu".into(),
            temperature_c: 42.0,
            recorded_at: Utc::now(),
        };
        let store = Arc::new(SqliteStore::in_memory().unwrap());
        let svc = test_service(store, Arc::new(MockCollector::new(vec![reading])), None);

        let result = svc
            .get_current_temperature(Parameters(GetCurrentTemperatureParams { auth_token: None }))
            .await
            .unwrap();

        let text = result.content[0].as_text().unwrap();
        assert!(text.text.contains("thermal_zone0:cpu"));
        assert!(text.text.contains("42"));
    }

    #[tokio::test]
    async fn get_current_temperature_denies_without_token() {
        let store = Arc::new(SqliteStore::in_memory().unwrap());
        let svc = test_service(
            store,
            Arc::new(MockCollector::new(vec![])),
            Some("secret".into()),
        );

        let err = svc
            .get_current_temperature(Parameters(GetCurrentTemperatureParams { auth_token: None }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("auth_token"));
    }

    #[tokio::test]
    async fn analyze_temperature_tool_returns_stats() {
        let store = Arc::new(SqliteStore::in_memory().unwrap());
        let t0 = Utc::now() - Duration::hours(1);
        store
            .insert_readings(&[SensorReading {
                sensor_name: "cpu".into(),
                temperature_c: 40.0,
                recorded_at: t0,
            }])
            .unwrap();

        let svc = test_service(store, Arc::new(MockCollector::new(vec![])), None);
        let from = (t0 - Duration::minutes(5)).to_rfc3339();
        let to = Utc::now().to_rfc3339();

        let result = svc
            .analyze_temperature(Parameters(AnalyzeTemperatureParams {
                from: Some(from),
                to: Some(to),
                sensor: Some("cpu".into()),
                auth_token: None,
            }))
            .await
            .unwrap();

        let text = result.content[0].as_text().unwrap();
        assert!(text.text.contains("\"min_c\":40"));
    }

    #[tokio::test]
    async fn analyze_temperature_rejects_invalid_range() {
        let store = Arc::new(SqliteStore::in_memory().unwrap());
        let svc = test_service(store, Arc::new(MockCollector::new(vec![])), None);
        let now = Utc::now().to_rfc3339();

        let err = svc
            .analyze_temperature(Parameters(AnalyzeTemperatureParams {
                from: Some(now.clone()),
                to: Some(now),
                sensor: None,
                auth_token: None,
            }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("from must be before to"));
    }

    #[test]
    fn server_info_lists_tools() {
        let store = Arc::new(SqliteStore::in_memory().unwrap());
        let svc = test_service(store, Arc::new(SysfsCollector::new()), None);
        let info = svc.get_info();
        assert!(info
            .instructions
            .unwrap()
            .contains("get_current_temperature@1"));
    }

    #[test]
    fn new_opens_audit_log_file() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("db.sqlite");
        let audit = dir.path().join("audit.jsonl");
        let svc = TemperatureMcpService::new(&db, Some(audit.clone())).unwrap();
        assert!(svc.authorize(None).is_ok());
    }

    #[tokio::test]
    async fn get_current_temperature_reports_collector_errors() {
        let store = Arc::new(SqliteStore::in_memory().unwrap());
        let svc = test_service(store, Arc::new(MockCollector::failing()), None);
        assert!(svc
            .get_current_temperature(Parameters(GetCurrentTemperatureParams { auth_token: None }))
            .await
            .is_err());
    }
}
