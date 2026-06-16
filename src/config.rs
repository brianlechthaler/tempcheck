use std::path::PathBuf;
use std::time::Duration;

use clap::{Parser, Subcommand};
use thiserror::Error;

pub const DEFAULT_DB_PATH: &str = "tempcheck.db";
pub const DEFAULT_INTERVAL_SECS: u64 = 30;
pub const DEFAULT_WEB_HOST: &str = "127.0.0.1";
pub const DEFAULT_WEB_PORT: u16 = 8080;
pub const DEFAULT_MCP_TOKEN_ENV: &str = "TEMPCHECK_MCP_TOKEN";
pub const MAX_ANALYSIS_ROWS: usize = 10_000;
pub const MAX_RESPONSE_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Parser)]
#[command(
    name = "tempcheck",
    about = "Monitor system temperatures and expose MCP tools"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Clone, Subcommand)]
pub enum Command {
    /// Run the background temperature logging daemon
    Daemon {
        /// SQLite database path
        #[arg(long, default_value = DEFAULT_DB_PATH)]
        db: PathBuf,
        /// Polling interval in seconds
        #[arg(long, default_value_t = DEFAULT_INTERVAL_SECS)]
        interval_secs: u64,
    },
    /// Start the MCP server (stdio transport)
    Mcp {
        /// SQLite database path (shared with daemon)
        #[arg(long, default_value = DEFAULT_DB_PATH)]
        db: PathBuf,
        /// JSONL audit log path (defaults to stderr-only audit events)
        #[arg(long)]
        audit_log: Option<PathBuf>,
    },
    /// Collect and print one snapshot (no daemon)
    Once {
        #[arg(long, default_value = DEFAULT_DB_PATH)]
        db: PathBuf,
        /// Also persist the reading
        #[arg(long)]
        save: bool,
    },
    /// Start web UI and JSON API for live/historical temperatures
    Web {
        /// SQLite database path (shared with daemon)
        #[arg(long, default_value = DEFAULT_DB_PATH)]
        db: PathBuf,
        /// HTTP bind host
        #[arg(long, default_value = DEFAULT_WEB_HOST)]
        host: String,
        /// HTTP bind port
        #[arg(long, default_value_t = DEFAULT_WEB_PORT)]
        port: u16,
    },
}

#[derive(Debug, Clone)]
pub struct Config {
    pub db_path: PathBuf,
    pub interval: Duration,
    pub audit_log: Option<PathBuf>,
    pub web_host: String,
    pub web_port: u16,
}

impl Config {
    pub fn from_cli(cli: Cli) -> Result<Self, ConfigError> {
        match cli.command {
            Command::Daemon { db, interval_secs } => {
                if interval_secs == 0 {
                    return Err(ConfigError::InvalidInterval);
                }
                Ok(Self {
                    db_path: db,
                    interval: Duration::from_secs(interval_secs),
                    audit_log: None,
                    web_host: DEFAULT_WEB_HOST.to_string(),
                    web_port: DEFAULT_WEB_PORT,
                })
            }
            Command::Mcp { db, audit_log } => Ok(Self {
                db_path: db,
                interval: Duration::from_secs(DEFAULT_INTERVAL_SECS),
                audit_log,
                web_host: DEFAULT_WEB_HOST.to_string(),
                web_port: DEFAULT_WEB_PORT,
            }),
            Command::Once { db, .. } => Ok(Self {
                db_path: db,
                interval: Duration::from_secs(DEFAULT_INTERVAL_SECS),
                audit_log: None,
                web_host: DEFAULT_WEB_HOST.to_string(),
                web_port: DEFAULT_WEB_PORT,
            }),
            Command::Web { db, host, port } => Ok(Self {
                db_path: db,
                interval: Duration::from_secs(DEFAULT_INTERVAL_SECS),
                audit_log: None,
                web_host: host,
                web_port: port,
            }),
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConfigError {
    #[error("interval must be greater than zero")]
    InvalidInterval,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daemon_config_rejects_zero_interval() {
        let cli = Cli {
            command: Command::Daemon {
                db: "x.db".into(),
                interval_secs: 0,
            },
        };
        assert_eq!(
            Config::from_cli(cli).unwrap_err(),
            ConfigError::InvalidInterval
        );
    }

    #[test]
    fn mcp_config_uses_db_path() {
        let cli = Cli {
            command: Command::Mcp {
                db: "custom.db".into(),
                audit_log: None,
            },
        };
        let cfg = Config::from_cli(cli).unwrap();
        assert_eq!(cfg.db_path, PathBuf::from("custom.db"));
    }

    #[test]
    fn once_config_uses_db_path() {
        let cli = Cli {
            command: Command::Once {
                db: "snap.db".into(),
                save: true,
            },
        };
        let cfg = Config::from_cli(cli).unwrap();
        assert_eq!(cfg.db_path, PathBuf::from("snap.db"));
    }

    #[test]
    fn web_config_uses_bind_target() {
        let cli = Cli {
            command: Command::Web {
                db: "web.db".into(),
                host: "0.0.0.0".into(),
                port: 9090,
            },
        };
        let cfg = Config::from_cli(cli).unwrap();
        assert_eq!(cfg.db_path, PathBuf::from("web.db"));
        assert_eq!(cfg.web_host, "0.0.0.0");
        assert_eq!(cfg.web_port, 9090);
    }
}
