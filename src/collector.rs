use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

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

/// Reads Linux hwmon sensors from `/sys/class/hwmon`.
#[derive(Debug, Clone, Default)]
pub struct HwmonCollector {
    hwmon_root: PathBuf,
}

impl HwmonCollector {
    pub fn new() -> Self {
        Self {
            hwmon_root: PathBuf::from("/sys/class/hwmon"),
        }
    }

    pub fn with_root(root: impl Into<PathBuf>) -> Self {
        Self {
            hwmon_root: root.into(),
        }
    }
}

impl TemperatureCollector for HwmonCollector {
    fn collect(&self) -> Result<Vec<SensorReading>, CollectorError> {
        if !self.hwmon_root.is_dir() {
            return Err(CollectorError::SysfsMissing(
                self.hwmon_root.display().to_string(),
            ));
        }

        let entries = fs::read_dir(&self.hwmon_root)
            .map_err(|e| CollectorError::Io(format!("read {}: {e}", self.hwmon_root.display())))?;
        let mut out = Vec::new();

        for entry in entries {
            let entry = entry.map_err(|e| CollectorError::Io(e.to_string()))?;
            let dir = entry.path();
            if !dir.is_dir() {
                continue;
            }

            let hwmon_name = fs::read_to_string(dir.join("name"))
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| {
                    dir.file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "unknown".to_string())
                });

            let files = fs::read_dir(&dir)
                .map_err(|e| CollectorError::Io(format!("read {}: {e}", dir.display())))?;
            for file_entry in files {
                let file_entry = file_entry.map_err(|e| CollectorError::Io(e.to_string()))?;
                let file_name = file_entry.file_name().to_string_lossy().into_owned();
                if !file_name.starts_with("temp") || !file_name.ends_with("_input") {
                    continue;
                }

                let sensor_id = file_name
                    .trim_start_matches("temp")
                    .trim_end_matches("_input")
                    .to_string();
                let label_path = dir.join(format!("temp{sensor_id}_label"));
                let label = fs::read_to_string(&label_path)
                    .ok()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| format!("temp{sensor_id}"));

                let input_path = file_entry.path();
                let raw = fs::read_to_string(&input_path)
                    .map_err(|e| CollectorError::Io(format!("read {}: {e}", input_path.display())))?
                    .trim()
                    .to_string();

                let milli_c: i64 = raw.parse().map_err(|_| CollectorError::InvalidReading {
                    path: input_path.display().to_string(),
                    value: raw,
                })?;

                out.push(SensorReading {
                    sensor_name: format!("hwmon:{hwmon_name}:{label}"),
                    temperature_c: milli_c as f64 / 1000.0,
                    recorded_at: Utc::now(),
                });
            }
        }

        out.sort_by(|a, b| a.sensor_name.cmp(&b.sensor_name));
        Ok(out)
    }
}

/// Reads NVIDIA GPU temperatures from the `nvidia-smi` CLI.
#[derive(Debug, Clone, Default)]
pub struct NvidiaSmiCollector {
    executable: PathBuf,
}

impl NvidiaSmiCollector {
    pub fn new() -> Self {
        Self {
            executable: PathBuf::from("nvidia-smi"),
        }
    }

    pub fn with_executable(path: impl Into<PathBuf>) -> Self {
        Self {
            executable: path.into(),
        }
    }
}

impl TemperatureCollector for NvidiaSmiCollector {
    fn collect(&self) -> Result<Vec<SensorReading>, CollectorError> {
        let output = Command::new(&self.executable)
            .args([
                "--query-gpu=index,name,temperature.gpu",
                "--format=csv,noheader,nounits",
            ])
            .output()
            .map_err(|e| CollectorError::Io(format!("exec {}: {e}", self.executable.display())))?;

        if !output.status.success() {
            return Err(CollectorError::Io(format!(
                "{} exited with status {status}",
                self.executable.display(),
                status = output.status
            )));
        }

        let stdout = String::from_utf8(output.stdout)
            .map_err(|e| CollectorError::Io(format!("parse nvidia-smi output: {e}")))?;
        let mut out = Vec::new();
        for line in stdout.lines().map(str::trim).filter(|line| !line.is_empty()) {
            let parts: Vec<&str> = line.split(',').map(|x| x.trim()).collect();
            if parts.len() < 3 {
                return Err(CollectorError::InvalidReading {
                    path: "nvidia-smi stdout".to_string(),
                    value: line.to_string(),
                });
            }
            let gpu_index = parts[0];
            let gpu_name = parts[1];
            let temp_raw = parts[2].to_string();
            let temperature_c: f64 = temp_raw.parse().map_err(|_| CollectorError::InvalidReading {
                path: "nvidia-smi stdout".to_string(),
                value: line.to_string(),
            })?;
            out.push(SensorReading {
                sensor_name: format!("nvidia:{gpu_index}:{gpu_name}"),
                temperature_c,
                recorded_at: Utc::now(),
            });
        }

        Ok(out)
    }
}

/// Best-effort collector that merges all available sensor backends.
#[derive(Debug, Clone, Default)]
pub struct SystemTemperatureCollector;

impl SystemTemperatureCollector {
    pub fn new() -> Self {
        Self
    }
}

impl TemperatureCollector for SystemTemperatureCollector {
    fn collect(&self) -> Result<Vec<SensorReading>, CollectorError> {
        let mut all = Vec::new();
        let mut errors = Vec::new();

        for collector in [
            Box::new(SysfsCollector::new()) as Box<dyn TemperatureCollector>,
            Box::new(HwmonCollector::new()),
            Box::new(NvidiaSmiCollector::new()),
        ] {
            match collector.collect() {
                Ok(mut readings) => all.append(&mut readings),
                Err(e) => errors.push(e),
            }
        }

        if !all.is_empty() {
            all.sort_by(|a, b| a.sensor_name.cmp(&b.sensor_name));
            return Ok(all);
        }

        let msg = if errors.is_empty() {
            "no temperature sensors found".to_string()
        } else {
            format!(
                "no sensor source succeeded: {}",
                errors
                    .iter()
                    .map(std::string::ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(" | ")
            )
        };
        Err(CollectorError::Io(msg))
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
    use std::os::unix::fs::PermissionsExt;
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

    #[test]
    fn hwmon_collector_reads_labeled_temps() {
        let tmp = TempDir::new().unwrap();
        let hwmon0 = tmp.path().join("hwmon0");
        fs::create_dir_all(&hwmon0).unwrap();
        fs::write(hwmon0.join("name"), "k10temp\n").unwrap();
        fs::write(hwmon0.join("temp1_label"), "Tctl\n").unwrap();
        fs::write(hwmon0.join("temp1_input"), "47000\n").unwrap();

        let collector = HwmonCollector::with_root(tmp.path());
        let readings = collector.collect().unwrap();
        assert_eq!(readings.len(), 1);
        assert_eq!(readings[0].sensor_name, "hwmon:k10temp:Tctl");
        assert!((readings[0].temperature_c - 47.0).abs() < f64::EPSILON);
    }

    #[test]
    fn nvidia_smi_collector_reads_cli_output() {
        let tmp = TempDir::new().unwrap();
        let script = tmp.path().join("nvidia-smi");
        fs::write(
            &script,
            "#!/usr/bin/env sh\necho \"0, NVIDIA RTX 4090, 53\"\necho \"1, NVIDIA RTX 4090, 55\"\n",
        )
        .unwrap();
        let mut perms = fs::metadata(&script).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script, perms).unwrap();

        let collector = NvidiaSmiCollector::with_executable(&script);
        let readings = collector.collect().unwrap();
        assert_eq!(readings.len(), 2);
        assert_eq!(readings[0].sensor_name, "nvidia:0:NVIDIA RTX 4090");
        assert!((readings[0].temperature_c - 53.0).abs() < f64::EPSILON);
        assert_eq!(readings[1].sensor_name, "nvidia:1:NVIDIA RTX 4090");
    }
}
