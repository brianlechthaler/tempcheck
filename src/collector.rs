use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SensorReading {
    pub sensor_name: String,
    pub temperature_c: f64,
    pub recorded_at: DateTime<Utc>,
}

pub trait TemperatureCollector: Send + Sync {
    fn collect(&self) -> Result<Vec<SensorReading>, CollectorError>;
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CollectorError {
    #[error("thermal sysfs path not found: {0}")]
    SysfsMissing(String),
    #[error("failed to read thermal data: {0}")]
    Io(String),
    #[error("invalid thermal reading in {path}: {value}")]
    InvalidReading { path: String, value: String },
}

/// Reads Linux thermal zones from `/sys/class/thermal`.
#[derive(Debug, Clone, Default)]
pub struct SysfsCollector {
    thermal_root: PathBuf,
}

impl SysfsCollector {
    pub fn new() -> Self {
        Self {
            thermal_root: PathBuf::from("/sys/class/thermal"),
        }
    }

    pub fn with_root(root: impl Into<PathBuf>) -> Self {
        Self {
            thermal_root: root.into(),
        }
    }

    fn zone_dirs(&self) -> Result<Vec<PathBuf>, CollectorError> {
        if !self.thermal_root.is_dir() {
            return Err(CollectorError::SysfsMissing(
                self.thermal_root.display().to_string(),
            ));
        }

        let mut zones = Vec::new();
        let entries = fs::read_dir(&self.thermal_root).map_err(|e| {
            CollectorError::Io(format!("read {}: {e}", self.thermal_root.display()))
        })?;

        for entry in entries {
            let entry = entry.map_err(|e| CollectorError::Io(e.to_string()))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("thermal_zone") {
                zones.push(entry.path());
            }
        }

        zones.sort();
        Ok(zones)
    }

    fn read_zone(&self, zone_path: &Path) -> Result<SensorReading, CollectorError> {
        let type_path = zone_path.join("type");
        let temp_path = zone_path.join("temp");

        let sensor_type = fs::read_to_string(&type_path)
            .map_err(|e| CollectorError::Io(format!("read {}: {e}", type_path.display())))?
            .trim()
            .to_string();

        let raw = fs::read_to_string(&temp_path)
            .map_err(|e| CollectorError::Io(format!("read {}: {e}", temp_path.display())))?
            .trim()
            .to_string();

        let milli_c: i64 = raw.parse().map_err(|_| CollectorError::InvalidReading {
            path: temp_path.display().to_string(),
            value: raw,
        })?;

        let zone_name = zone_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "unknown".to_string());

        Ok(SensorReading {
            sensor_name: format!("{zone_name}:{sensor_type}"),
            temperature_c: milli_c as f64 / 1000.0,
            recorded_at: Utc::now(),
        })
    }
}

impl TemperatureCollector for SysfsCollector {
    fn collect(&self) -> Result<Vec<SensorReading>, CollectorError> {
        let zones = self.zone_dirs()?;
        zones
            .iter()
            .map(|z| self.read_zone(z))
            .collect::<Result<Vec<_>, _>>()
    }
}

/// Deterministic collector for unit tests.
#[derive(Debug, Clone)]
pub struct MockCollector {
    readings: Vec<SensorReading>,
    fail: bool,
}

impl MockCollector {
    pub fn new(readings: Vec<SensorReading>) -> Self {
        Self {
            readings,
            fail: false,
        }
    }

    pub fn failing() -> Self {
        Self {
            readings: Vec::new(),
            fail: true,
        }
    }
}

impl TemperatureCollector for MockCollector {
    fn collect(&self) -> Result<Vec<SensorReading>, CollectorError> {
        if self.fail {
            return Err(CollectorError::Io("mock failure".to_string()));
        }
        Ok(self.readings.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write_zone(dir: &Path, zone: &str, sensor_type: &str, milli_c: &str) {
        let zone_dir = dir.join(zone);
        fs::create_dir_all(&zone_dir).unwrap();
        fs::write(zone_dir.join("type"), sensor_type).unwrap();
        fs::write(zone_dir.join("temp"), milli_c).unwrap();
    }

    #[test]
    fn sysfs_collector_reads_zones() {
        let tmp = TempDir::new().unwrap();
        write_zone(tmp.path(), "thermal_zone0", "cpu", "42000");
        write_zone(tmp.path(), "thermal_zone1", "gpu", "55000");

        let collector = SysfsCollector::with_root(tmp.path());
        let readings = collector.collect().unwrap();

        assert_eq!(readings.len(), 2);
        assert_eq!(readings[0].sensor_name, "thermal_zone0:cpu");
        assert!((readings[0].temperature_c - 42.0).abs() < f64::EPSILON);
        assert_eq!(readings[1].sensor_name, "thermal_zone1:gpu");
        assert!((readings[1].temperature_c - 55.0).abs() < f64::EPSILON);
    }

    #[test]
    fn sysfs_collector_errors_on_missing_root() {
        let collector = SysfsCollector::with_root("/nonexistent/thermal/path");
        let err = collector.collect().unwrap_err();
        assert_eq!(
            err,
            CollectorError::SysfsMissing("/nonexistent/thermal/path".to_string())
        );
    }

    #[test]
    fn sysfs_collector_errors_on_invalid_temp() {
        let tmp = TempDir::new().unwrap();
        write_zone(tmp.path(), "thermal_zone0", "cpu", "not-a-number");

        let collector = SysfsCollector::with_root(tmp.path());
        let err = collector.collect().unwrap_err();
        assert!(matches!(err, CollectorError::InvalidReading { .. }));
    }

    #[test]
    fn mock_collector_returns_configured_readings() {
        let reading = SensorReading {
            sensor_name: "test:cpu".to_string(),
            temperature_c: 37.5,
            recorded_at: Utc::now(),
        };
        let collector = MockCollector::new(vec![reading.clone()]);
        let out = collector.collect().unwrap();
        assert_eq!(out, vec![reading]);
    }

    #[test]
    fn mock_collector_can_fail() {
        let collector = MockCollector::failing();
        assert!(collector.collect().is_err());
    }
}
