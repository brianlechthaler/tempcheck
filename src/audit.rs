use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex};

use chrono::Utc;
use serde::Serialize;
use sha2::{Digest, Sha256};

/// Structured SIEM-ready audit event for MCP tool invocations.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AuditEvent {
    pub timestamp: String,
    pub event_type: String,
    pub tool_name: String,
    pub tool_version: String,
    pub caller_identity: String,
    pub authorization_scope: String,
    pub params_fingerprint: String,
    pub result_status: String,
    pub result_bytes: usize,
    pub correlation_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl AuditEvent {
    pub fn invoke(
        tool_name: &str,
        tool_version: &str,
        caller: &str,
        scope: &str,
        params_json: &str,
        correlation_id: &str,
    ) -> Self {
        Self {
            timestamp: Utc::now().to_rfc3339(),
            event_type: "mcp.tool.invoke".to_string(),
            tool_name: tool_name.to_string(),
            tool_version: tool_version.to_string(),
            caller_identity: caller.to_string(),
            authorization_scope: scope.to_string(),
            params_fingerprint: fingerprint(params_json),
            result_status: "pending".to_string(),
            result_bytes: 0,
            correlation_id: correlation_id.to_string(),
            error: None,
        }
    }

    pub fn with_result(mut self, status: &str, result_bytes: usize, error: Option<String>) -> Self {
        self.event_type = "mcp.tool.result".to_string();
        self.result_status = status.to_string();
        self.result_bytes = result_bytes;
        self.error = error;
        self
    }
}

pub fn fingerprint(data: &str) -> String {
    let hash = Sha256::digest(data.as_bytes());
    hex::encode(hash)
}

pub trait AuditSink: Send + Sync {
    fn emit(&self, event: &AuditEvent);
}

#[derive(Debug, Default)]
pub struct StderrAuditSink;

impl AuditSink for StderrAuditSink {
    fn emit(&self, event: &AuditEvent) {
        if let Ok(line) = serde_json::to_string(event) {
            eprintln!("{line}");
        }
    }
}

#[derive(Debug)]
pub struct FileAuditSink {
    file: Arc<Mutex<std::fs::File>>,
}

impl FileAuditSink {
    pub fn open(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        Ok(Self {
            file: Arc::new(Mutex::new(file)),
        })
    }
}

impl AuditSink for FileAuditSink {
    fn emit(&self, event: &AuditEvent) {
        if let Ok(line) = serde_json::to_string(event) {
            if let Ok(mut file) = self.file.lock() {
                let _ = writeln!(file, "{line}");
            }
        }
    }
}

pub struct MultiAuditSink {
    sinks: Vec<Arc<dyn AuditSink>>,
}

impl MultiAuditSink {
    pub fn new(sinks: Vec<Arc<dyn AuditSink>>) -> Self {
        Self { sinks }
    }
}

impl AuditSink for MultiAuditSink {
    fn emit(&self, event: &AuditEvent) {
        for sink in &self.sinks {
            sink.emit(event);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_is_deterministic() {
        let a = fingerprint("{\"sensor\":\"cpu\"}");
        let b = fingerprint("{\"sensor\":\"cpu\"}");
        let c = fingerprint("{\"sensor\":\"gpu\"}");
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(a.len(), 64);
    }

    #[test]
    fn audit_event_serializes_required_fields() {
        let event = AuditEvent::invoke(
            "get_current_temperature",
            "1",
            "agent:test",
            "temperature:read",
            "{}",
            "corr-1",
        )
        .with_result("success", 128, None);

        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["event_type"], "mcp.tool.result");
        assert_eq!(json["tool_name"], "get_current_temperature");
        assert_eq!(json["result_status"], "success");
    }

    #[test]
    fn file_sink_writes_jsonl() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.jsonl");
        let sink = FileAuditSink::open(&path).unwrap();
        let event = AuditEvent::invoke("t", "1", "c", "s", "{}", "id");
        sink.emit(&event);
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains("mcp.tool.invoke"));
    }

    #[test]
    fn multi_sink_emits_to_all() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.jsonl");
        let file = Arc::new(FileAuditSink::open(&path).unwrap());
        let multi = MultiAuditSink::new(vec![Arc::new(StderrAuditSink), file]);
        let event = AuditEvent::invoke("t", "1", "c", "s", "{}", "id");
        multi.emit(&event);
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains("mcp.tool.invoke"));
    }
}
