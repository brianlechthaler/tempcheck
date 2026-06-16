use std::path::Path;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::collector::SensorReading;
use crate::config::MAX_ANALYSIS_ROWS;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AnalysisResult {
    pub sensor_name: String,
    pub count: u64,
    pub min_c: f64,
    pub max_c: f64,
    pub avg_c: f64,
    pub latest_c: f64,
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum StoreError {
    #[error("database error: {0}")]
    Db(String),
    #[error("invalid time range: start must be before end")]
    InvalidRange,
    #[error("row limit exceeded (max {MAX_ANALYSIS_ROWS})")]
    RowLimitExceeded,
}

pub trait TemperatureStore: Send + Sync {
    fn insert_readings(&self, readings: &[SensorReading]) -> Result<usize, StoreError>;
    fn analyze(
        &self,
        sensor: Option<&str>,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<AnalysisResult>, StoreError>;
    fn recent_readings(
        &self,
        sensor: Option<&str>,
        limit: usize,
    ) -> Result<Vec<SensorReading>, StoreError>;
}

#[derive(Debug, Clone)]
pub struct SqliteStore {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let conn = Connection::open(path).map_err(|e| StoreError::Db(e.to_string()))?;
        Self::init_schema(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub fn in_memory() -> Result<Self, StoreError> {
        let conn = Connection::open_in_memory().map_err(|e| StoreError::Db(e.to_string()))?;
        Self::init_schema(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    fn init_schema(conn: &Connection) -> Result<(), StoreError> {
        conn.execute_batch(
            "
            PRAGMA journal_mode=WAL;
            CREATE TABLE IF NOT EXISTS temperature_readings (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                sensor_name TEXT NOT NULL,
                temperature_c REAL NOT NULL,
                recorded_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_readings_recorded_at
                ON temperature_readings(recorded_at);
            CREATE INDEX IF NOT EXISTS idx_readings_sensor
                ON temperature_readings(sensor_name);
            ",
        )
        .map_err(|e| StoreError::Db(e.to_string()))
    }
}

impl TemperatureStore for SqliteStore {
    fn insert_readings(&self, readings: &[SensorReading]) -> Result<usize, StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::Db(e.to_string()))?;
        let mut count = 0usize;
        for reading in readings {
            conn.execute(
                "INSERT INTO temperature_readings (sensor_name, temperature_c, recorded_at)
                 VALUES (?1, ?2, ?3)",
                params![
                    reading.sensor_name,
                    reading.temperature_c,
                    reading.recorded_at.to_rfc3339(),
                ],
            )
            .map_err(|e| StoreError::Db(e.to_string()))?;
            count += 1;
        }
        Ok(count)
    }

    fn analyze(
        &self,
        sensor: Option<&str>,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<AnalysisResult>, StoreError> {
        if from >= to {
            return Err(StoreError::InvalidRange);
        }

        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::Db(e.to_string()))?;

        let count: i64 = match sensor {
            Some(name) => conn
                .query_row(
                    "SELECT COUNT(*) FROM temperature_readings
                     WHERE sensor_name = ?1 AND recorded_at >= ?2 AND recorded_at <= ?3",
                    params![name, from.to_rfc3339(), to.to_rfc3339()],
                    |row| row.get(0),
                )
                .map_err(|e| StoreError::Db(e.to_string()))?,
            None => conn
                .query_row(
                    "SELECT COUNT(*) FROM temperature_readings
                     WHERE recorded_at >= ?1 AND recorded_at <= ?2",
                    params![from.to_rfc3339(), to.to_rfc3339()],
                    |row| row.get(0),
                )
                .map_err(|e| StoreError::Db(e.to_string()))?,
        };

        if count as usize > MAX_ANALYSIS_ROWS {
            return Err(StoreError::RowLimitExceeded);
        }

        let sql = if sensor.is_some() {
            "
            SELECT sensor_name,
                   COUNT(*) as cnt,
                   MIN(temperature_c) as min_c,
                   MAX(temperature_c) as max_c,
                   AVG(temperature_c) as avg_c,
                   (SELECT temperature_c FROM temperature_readings t2
                    WHERE t2.sensor_name = t1.sensor_name
                      AND t2.recorded_at >= ?2 AND t2.recorded_at <= ?3
                    ORDER BY t2.recorded_at DESC LIMIT 1) as latest_c,
                   MIN(recorded_at) as from_ts,
                   MAX(recorded_at) as to_ts
            FROM temperature_readings t1
            WHERE sensor_name = ?1 AND recorded_at >= ?2 AND recorded_at <= ?3
            GROUP BY sensor_name
            "
        } else {
            "
            SELECT sensor_name,
                   COUNT(*) as cnt,
                   MIN(temperature_c) as min_c,
                   MAX(temperature_c) as max_c,
                   AVG(temperature_c) as avg_c,
                   (SELECT temperature_c FROM temperature_readings t2
                    WHERE t2.sensor_name = t1.sensor_name
                      AND t2.recorded_at >= ?1 AND t2.recorded_at <= ?2
                    ORDER BY t2.recorded_at DESC LIMIT 1) as latest_c,
                   MIN(recorded_at) as from_ts,
                   MAX(recorded_at) as to_ts
            FROM temperature_readings t1
            WHERE recorded_at >= ?1 AND recorded_at <= ?2
            GROUP BY sensor_name
            "
        };

        let mut stmt = conn
            .prepare(sql)
            .map_err(|e| StoreError::Db(e.to_string()))?;

        let map_row = |row: &rusqlite::Row<'_>| {
            let from_str: String = row.get(6)?;
            let to_str: String = row.get(7)?;
            Ok(AnalysisResult {
                sensor_name: row.get(0)?,
                count: row.get::<_, i64>(1)? as u64,
                min_c: row.get(2)?,
                max_c: row.get(3)?,
                avg_c: row.get(4)?,
                latest_c: row.get(5)?,
                from: DateTime::parse_from_rfc3339(&from_str)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?
                    .with_timezone(&Utc),
                to: DateTime::parse_from_rfc3339(&to_str)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?
                    .with_timezone(&Utc),
            })
        };

        let results = if let Some(name) = sensor {
            stmt.query_map(params![name, from.to_rfc3339(), to.to_rfc3339()], map_row)
        } else {
            stmt.query_map(params![from.to_rfc3339(), to.to_rfc3339()], map_row)
        }
        .map_err(|e| StoreError::Db(e.to_string()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| StoreError::Db(e.to_string()))?;

        Ok(results)
    }

    fn recent_readings(
        &self,
        sensor: Option<&str>,
        limit: usize,
    ) -> Result<Vec<SensorReading>, StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::Db(e.to_string()))?;

        let sql = if sensor.is_some() {
            "SELECT sensor_name, temperature_c, recorded_at
             FROM temperature_readings
             WHERE sensor_name = ?1
             ORDER BY recorded_at DESC
             LIMIT ?2"
        } else {
            "SELECT sensor_name, temperature_c, recorded_at
             FROM temperature_readings
             ORDER BY recorded_at DESC
             LIMIT ?1"
        };

        let mut stmt = conn
            .prepare(sql)
            .map_err(|e| StoreError::Db(e.to_string()))?;

        let rows = if let Some(name) = sensor {
            stmt.query_map(params![name, limit as i64], map_row_reading)
        } else {
            stmt.query_map(params![limit as i64], map_row_reading)
        }
        .map_err(|e| StoreError::Db(e.to_string()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| StoreError::Db(e.to_string()))?;

        Ok(rows)
    }
}

fn map_row_reading(row: &rusqlite::Row<'_>) -> rusqlite::Result<SensorReading> {
    let recorded_at: String = row.get(2)?;
    Ok(SensorReading {
        sensor_name: row.get(0)?,
        temperature_c: row.get(1)?,
        recorded_at: DateTime::parse_from_rfc3339(&recorded_at)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?
            .with_timezone(&Utc),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn sample_reading(name: &str, temp: f64, at: DateTime<Utc>) -> SensorReading {
        SensorReading {
            sensor_name: name.to_string(),
            temperature_c: temp,
            recorded_at: at,
        }
    }

    #[test]
    fn insert_and_analyze() {
        let store = SqliteStore::in_memory().unwrap();
        let t0 = Utc::now() - Duration::hours(2);
        let t1 = t0 + Duration::minutes(30);
        let t2 = t0 + Duration::hours(1);

        store
            .insert_readings(&[
                sample_reading("cpu", 40.0, t0),
                sample_reading("cpu", 50.0, t1),
                sample_reading("gpu", 60.0, t2),
            ])
            .unwrap();

        let from = t0 - Duration::minutes(1);
        let to = t2 + Duration::minutes(1);
        let all = store.analyze(None, from, to).unwrap();
        assert_eq!(all.len(), 2);

        let cpu = store.analyze(Some("cpu"), from, to).unwrap();
        assert_eq!(cpu.len(), 1);
        assert_eq!(cpu[0].count, 2);
        assert!((cpu[0].min_c - 40.0).abs() < f64::EPSILON);
        assert!((cpu[0].max_c - 50.0).abs() < f64::EPSILON);
        assert!((cpu[0].avg_c - 45.0).abs() < f64::EPSILON);
        assert!((cpu[0].latest_c - 50.0).abs() < f64::EPSILON);
    }

    #[test]
    fn analyze_rejects_invalid_range() {
        let store = SqliteStore::in_memory().unwrap();
        let now = Utc::now();
        let err = store.analyze(None, now, now).unwrap_err();
        assert_eq!(err, StoreError::InvalidRange);
    }

    #[test]
    fn recent_readings_respects_limit_and_filter() {
        let store = SqliteStore::in_memory().unwrap();
        let t0 = Utc::now();
        store
            .insert_readings(&[
                sample_reading("cpu", 40.0, t0),
                sample_reading("cpu", 41.0, t0 + Duration::seconds(1)),
                sample_reading("gpu", 55.0, t0 + Duration::seconds(2)),
            ])
            .unwrap();

        let recent = store.recent_readings(Some("cpu"), 1).unwrap();
        assert_eq!(recent.len(), 1);
        assert!((recent[0].temperature_c - 41.0).abs() < f64::EPSILON);

        let all = store.recent_readings(None, 2).unwrap();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn open_creates_persistent_db() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");
        {
            let store = SqliteStore::open(&path).unwrap();
            store
                .insert_readings(&[sample_reading("cpu", 42.0, Utc::now())])
                .unwrap();
        }
        let store = SqliteStore::open(&path).unwrap();
        let rows = store.recent_readings(None, 10).unwrap();
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn analyze_all_sensors_empty_range() {
        let store = SqliteStore::in_memory().unwrap();
        let from = Utc::now() - Duration::hours(1);
        let to = Utc::now();
        let results = store.analyze(None, from, to).unwrap();
        assert!(results.is_empty());
    }
}
