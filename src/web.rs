use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Result;
use axum::extract::{Query, State};
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use tracing::info;

use crate::collector::{SensorReading, SystemTemperatureCollector, TemperatureCollector};
use crate::storage::{AnalysisResult, StoreError, TemperatureStore};

#[derive(Clone)]
struct WebState {
    store: Arc<dyn TemperatureStore>,
    collector: Arc<dyn TemperatureCollector>,
}

#[derive(Debug, Deserialize)]
struct HistoryQuery {
    sensor: Option<String>,
    hours: Option<i64>,
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    error: String,
}

pub async fn run_web_server(
    store: Arc<crate::storage::SqliteStore>,
    host: &str,
    port: u16,
) -> Result<()> {
    let state = WebState {
        store,
        collector: Arc::new(SystemTemperatureCollector::new()),
    };
    let app = router(state);
    let addr: SocketAddr = format!("{host}:{port}").parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!(address = %addr, "starting web UI server");
    axum::serve(listener, app).await?;
    Ok(())
}

fn router(state: WebState) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/api/current", get(get_current))
        .route("/api/history", get(get_history))
        .with_state(state)
}

async fn index() -> impl IntoResponse {
    let mut resp = Html(INDEX_HTML).into_response();
    resp.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    resp
}

async fn get_current(State(state): State<WebState>) -> Result<Json<Vec<SensorReading>>, ApiError> {
    let mut readings =
        latest_sensor_readings(state.store.as_ref(), 512).map_err(map_store_error)?;
    if readings.is_empty() {
        // Fallback for web-only mode with no daemon writes yet.
        readings = state
            .collector
            .collect()
            .map_err(|e| ApiError::internal(e.to_string()))?;
        if !readings.is_empty() {
            state
                .store
                .insert_readings(&readings)
                .map_err(map_store_error)?;
        }
    }
    Ok(Json(readings))
}

async fn get_history(
    State(state): State<WebState>,
    Query(query): Query<HistoryQuery>,
) -> Result<Json<Vec<AnalysisResult>>, ApiError> {
    let hours = query.hours.unwrap_or(24);
    if !(1..=24 * 30).contains(&hours) {
        return Err(ApiError::bad_request(
            "hours must be between 1 and 720".to_string(),
        ));
    }

    let to = Utc::now();
    let from = to - Duration::hours(hours);
    let sensor = query.sensor.as_deref();
    let out = state
        .store
        .analyze(sensor, from, to)
        .map_err(map_store_error)?;
    Ok(Json(out))
}

fn map_store_error(e: StoreError) -> ApiError {
    match e {
        StoreError::InvalidRange | StoreError::RowLimitExceeded => {
            ApiError::bad_request(e.to_string())
        }
        StoreError::Db(msg) => ApiError::internal(msg),
    }
}

fn latest_sensor_readings(
    store: &dyn TemperatureStore,
    row_limit: usize,
) -> Result<Vec<SensorReading>, StoreError> {
    let recent = store.recent_readings(None, row_limit)?;
    let mut seen = HashSet::new();
    let mut latest = Vec::new();
    for reading in recent {
        if seen.insert(reading.sensor_name.clone()) {
            latest.push(reading);
        }
    }
    latest.sort_by(|a, b| a.sensor_name.cmp(&b.sensor_name));
    Ok(latest)
}

struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn bad_request(message: String) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message,
        }
    }

    fn internal(message: String) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message,
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ErrorBody {
                error: self.message,
            }),
        )
            .into_response()
    }
}

const INDEX_HTML: &str = r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>tempcheck monitor</title>
  <style>
    :root { color-scheme: light dark; font-family: ui-sans-serif, system-ui, -apple-system, Segoe UI, Roboto, sans-serif; }
    body { margin: 0; padding: 1rem; background: #0b1020; color: #e8ecf3; }
    .wrap { max-width: 980px; margin: 0 auto; }
    h1 { margin: 0 0 0.5rem; font-size: 1.4rem; }
    p { margin-top: 0; color: #a5b3c7; }
    .grid { display: grid; gap: 1rem; grid-template-columns: repeat(auto-fit, minmax(270px, 1fr)); }
    .card { border: 1px solid #2a3550; border-radius: 10px; padding: 0.9rem; background: #121a30; }
    .metric { font-size: 1.7rem; font-weight: 700; margin: 0.2rem 0; }
    .muted { color: #91a0b5; font-size: 0.9rem; }
    .controls { display: flex; gap: 0.6rem; align-items: center; flex-wrap: wrap; margin-bottom: 0.8rem; }
    .controls label { min-width: 8rem; font-weight: 600; }
    select, button { background: #1b2744; border: 1px solid #344264; color: #e8ecf3; border-radius: 8px; padding: 0.45rem 0.7rem; }
    button { cursor: pointer; }
    table { width: 100%; border-collapse: collapse; margin-top: 0.8rem; font-size: 0.95rem; }
    th, td { border-bottom: 1px solid #2a3550; padding: 0.5rem; text-align: left; }
    th:nth-child(n+2), td:nth-child(n+2) { text-align: right; font-variant-numeric: tabular-nums; }
    #status { margin-top: 0.8rem; color: #9fb0ca; min-height: 1.2rem; }
    .chart { display: block; width: 100%; height: 220px; margin-top: 0.5rem; border: 1px solid #2a3550; border-radius: 8px; background: #0f1730; }
    .axis { stroke: #5a6c93; stroke-width: 1; }
    .axis-label { fill: #9fb0ca; font-size: 13px; font-weight: 600; }
    .tick-label { fill: #9fb0ca; font-size: 12px; }
    .line { stroke: #4ac1ff; stroke-width: 2; fill: none; }
    .point { fill: #7fd3ff; }
    .hover-marker { fill: #ffd166; stroke: #0f1730; stroke-width: 1.5; }
    .hover-line { stroke: #ffd166; stroke-width: 1; stroke-dasharray: 3 3; }
    .chart-overlay { fill: transparent; cursor: crosshair; }
    .chart-readout { margin-top: 0.45rem; color: #9fb0ca; min-height: 1.1rem; font-size: 0.92rem; }
  </style>
</head>
<body>
  <div class="wrap">
    <h1>Temperature monitor</h1>
    <p>Live sensor snapshot and historical summaries from tempcheck.</p>
    <div class="controls">
      <label for="hours">History window</label>
      <select id="hours">
        <option value="1">1h</option>
        <option value="6">6h</option>
        <option value="24" selected>24h</option>
        <option value="72">72h</option>
      </select>
      <button id="refresh">Refresh now</button>
    </div>
    <div id="live" class="grid"></div>
    <div class="card">
      <div class="muted">Avg temperature trend by sensor</div>
      <svg id="chart" class="chart" viewBox="0 0 600 160" preserveAspectRatio="none"></svg>
      <div id="chart-readout" class="chart-readout">Hover chart to inspect sensor avg values.</div>
      <table>
        <thead>
          <tr><th>Sensor</th><th>Latest (C)</th><th>Min (C)</th><th>Max (C)</th><th>Avg (C)</th><th>Samples</th></tr>
        </thead>
        <tbody id="history-body"></tbody>
      </table>
    </div>
    <div id="status"></div>
  </div>
<script>
const liveEl = document.getElementById('live');
const historyBody = document.getElementById('history-body');
const statusEl = document.getElementById('status');
const hoursEl = document.getElementById('hours');
const chartEl = document.getElementById('chart');
const chartReadoutEl = document.getElementById('chart-readout');
const refreshBtn = document.getElementById('refresh');
let chartPoints = [];

function setStatus(msg) {
  statusEl.textContent = msg;
}

function fmt(n) {
  return Number(n).toFixed(1);
}

function esc(text) {
  return String(text)
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;')
    .replaceAll("'", '&#39;');
}

function renderLive(items) {
  liveEl.innerHTML = '';
  if (!items.length) {
    liveEl.innerHTML = '<div class="card"><div class="muted">No live readings available.</div></div>';
    return;
  }
  for (const item of items) {
    const card = document.createElement('div');
    card.className = 'card';
    card.innerHTML = `<div class="muted">${esc(item.sensor_name)}</div><div class="metric">${fmt(item.temperature_c)} C</div><div class="muted">${new Date(item.recorded_at).toLocaleString()}</div>`;
    liveEl.appendChild(card);
  }
}

function bindChartHover() {
  const overlay = document.getElementById('chart-overlay');
  if (!overlay) return;
  const marker = document.getElementById('hover-marker');
  const hoverLine = document.getElementById('hover-line');
  overlay.addEventListener('mousemove', (ev) => {
    if (!chartPoints.length || !marker || !hoverLine) return;
    const bounds = overlay.getBoundingClientRect();
    const x = ((ev.clientX - bounds.left) / bounds.width) * 600;
    let nearest = chartPoints[0];
    for (const point of chartPoints) {
      if (Math.abs(point.x - x) < Math.abs(nearest.x - x)) {
        nearest = point;
      }
    }
    marker.setAttribute('cx', nearest.x);
    marker.setAttribute('cy', nearest.y);
    marker.style.display = 'block';
    hoverLine.setAttribute('x1', nearest.x);
    hoverLine.setAttribute('x2', nearest.x);
    hoverLine.style.display = 'block';
    chartReadoutEl.textContent = `${nearest.sensor}: avg ${fmt(nearest.avg)} C (min ${fmt(nearest.min)} C, max ${fmt(nearest.max)} C, samples ${nearest.count})`;
  });
  overlay.addEventListener('mouseleave', () => {
    if (!marker || !hoverLine) return;
    marker.style.display = 'none';
    hoverLine.style.display = 'none';
    chartReadoutEl.textContent = 'Hover chart to inspect sensor avg values.';
  });
}

function renderHistory(items) {
  historyBody.innerHTML = '';
  if (!items.length) {
    historyBody.innerHTML = '<tr><td colspan="6" class="muted">No historical data found for selected window.</td></tr>';
    chartEl.innerHTML = '';
    chartPoints = [];
    chartReadoutEl.textContent = 'No trend data available for current selection.';
    return;
  }
  for (const item of items) {
    const tr = document.createElement('tr');
    tr.innerHTML = `<td>${esc(item.sensor_name)}</td><td>${fmt(item.latest_c)}</td><td>${fmt(item.min_c)}</td><td>${fmt(item.max_c)}</td><td>${fmt(item.avg_c)}</td><td>${item.count}</td>`;
    historyBody.appendChild(tr);
  }

  const left = 54;
  const right = 20;
  const top = 18;
  const bottom = 42;
  const width = 600 - left - right;
  const height = 160 - top - bottom;
  const min = Math.min(...items.map((x) => x.min_c));
  const max = Math.max(...items.map((x) => x.max_c));
  const span = Math.max(max - min, 0.5);
  chartPoints = items.map((x, i) => {
    const px = left + (i / Math.max(items.length - 1, 1)) * width;
    const py = top + (1 - (x.avg_c - min) / span) * height;
    return { x: px, y: py, sensor: x.sensor_name, avg: x.avg_c, min: x.min_c, max: x.max_c, count: x.count };
  });
  const linePoints = chartPoints.map((p) => `${p.x},${p.y}`).join(' ');
  const circles = chartPoints
    .map((p) => `<circle class="point" cx="${p.x}" cy="${p.y}" r="3"></circle>`)
    .join('');
  const xTicks = chartPoints
    .map((p) => `<text class="tick-label" x="${p.x}" y="${top + height + 16}" text-anchor="middle">${esc(p.sensor)}</text>`)
    .join('');
  const yTop = max;
  const yMid = min + span / 2;
  const yBottom = min;
  chartEl.innerHTML = `
    <line class="axis" x1="${left}" y1="${top}" x2="${left}" y2="${top + height}"></line>
    <line class="axis" x1="${left}" y1="${top + height}" x2="${left + width}" y2="${top + height}"></line>
    <text class="axis-label" x="${left + width / 2}" y="156" text-anchor="middle">Sensor</text>
    <text class="axis-label" x="16" y="${top + height / 2}" text-anchor="middle" transform="rotate(-90 16 ${top + height / 2})">Avg temp (C)</text>
    <text class="tick-label" x="${left - 8}" y="${top + 4}" text-anchor="end">${fmt(yTop)}</text>
    <text class="tick-label" x="${left - 8}" y="${top + height / 2 + 4}" text-anchor="end">${fmt(yMid)}</text>
    <text class="tick-label" x="${left - 8}" y="${top + height + 4}" text-anchor="end">${fmt(yBottom)}</text>
    <polyline class="line" points="${linePoints}"></polyline>
    ${circles}
    ${xTicks}
    <line id="hover-line" class="hover-line" x1="${left}" y1="${top}" x2="${left}" y2="${top + height}" style="display:none"></line>
    <circle id="hover-marker" class="hover-marker" cx="${left}" cy="${top + height}" r="4" style="display:none"></circle>
    <rect id="chart-overlay" class="chart-overlay" x="${left}" y="${top}" width="${width}" height="${height}"></rect>
  `;
  bindChartHover();
  chartReadoutEl.textContent = 'Hover chart to inspect sensor avg values.';
}

async function loadAll() {
  const hours = Number(hoursEl.value);
  setStatus('Loading...');
  const [liveResp, histResp] = await Promise.all([
    fetch('/api/current'),
    fetch(`/api/history?hours=${hours}`)
  ]);
  if (!liveResp.ok) throw new Error('Failed to load live data');
  if (!histResp.ok) throw new Error('Failed to load history');
  const live = await liveResp.json();
  const hist = await histResp.json();
  renderLive(live);
  renderHistory(hist);
  setStatus(`Updated at ${new Date().toLocaleTimeString()}`);
}

refreshBtn.addEventListener('click', () => {
  loadAll().catch((e) => setStatus(e.message));
});
hoursEl.addEventListener('change', () => {
  loadAll().catch((e) => setStatus(e.message));
});
setInterval(() => {
  loadAll().catch((e) => setStatus(e.message));
}, 15000);
loadAll().catch((e) => setStatus(e.message));
</script>
</body>
</html>
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use axum::http::Request;
    use chrono::Duration;
    use tower::util::ServiceExt;

    use crate::collector::MockCollector;
    use crate::collector::SensorReading;
    use crate::storage::{SqliteStore, TemperatureStore};
    use chrono::Utc;

    #[tokio::test]
    async fn index_returns_html() {
        let state = WebState {
            store: Arc::new(SqliteStore::in_memory().unwrap()),
            collector: Arc::new(MockCollector::new(vec![])),
        };
        let app = router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
        let body = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(body.contains("chart-readout"));
        assert!(body.contains("Avg temp (C)"));
    }

    #[tokio::test]
    async fn history_validates_hours() {
        let state = WebState {
            store: Arc::new(SqliteStore::in_memory().unwrap()),
            collector: Arc::new(MockCollector::new(vec![])),
        };
        let app = router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/history?hours=0")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let bytes = to_bytes(response.into_body(), 16 * 1024).await.unwrap();
        let body = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(body.contains("hours must be between"));
    }

    #[tokio::test]
    async fn current_returns_readings() {
        let state = WebState {
            store: Arc::new(SqliteStore::in_memory().unwrap()),
            collector: Arc::new(MockCollector::new(vec![SensorReading {
                sensor_name: "cpu".to_string(),
                temperature_c: 44.2,
                recorded_at: Utc::now(),
            }])),
        };
        let app = router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/current")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), 16 * 1024).await.unwrap();
        let body = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(body.contains("cpu"));
        assert!(body.contains("44.2"));
    }

    #[tokio::test]
    async fn current_returns_internal_error_on_collector_failure() {
        let state = WebState {
            store: Arc::new(SqliteStore::in_memory().unwrap()),
            collector: Arc::new(MockCollector::failing()),
        };
        let app = router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/current")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn history_returns_aggregates() {
        let store = Arc::new(SqliteStore::in_memory().unwrap());
        store
            .insert_readings(&[SensorReading {
                sensor_name: "cpu".to_string(),
                temperature_c: 42.0,
                recorded_at: Utc::now(),
            }])
            .unwrap();

        let state = WebState {
            store,
            collector: Arc::new(MockCollector::new(vec![])),
        };
        let app = router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/history?hours=24")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), 16 * 1024).await.unwrap();
        let body = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(body.contains("cpu"));
        assert!(body.contains("latest_c"));
    }

    #[tokio::test]
    async fn current_persists_readings_for_history() {
        let store = Arc::new(SqliteStore::in_memory().unwrap());
        let state = WebState {
            store: store.clone(),
            collector: Arc::new(MockCollector::new(vec![SensorReading {
                sensor_name: "cpu".to_string(),
                temperature_c: 41.5,
                recorded_at: Utc::now(),
            }])),
        };
        let app = router(state);

        let current_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/current")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(current_response.status(), StatusCode::OK);

        let history_response = app
            .oneshot(
                Request::builder()
                    .uri("/api/history?hours=1")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(history_response.status(), StatusCode::OK);
        let bytes = to_bytes(history_response.into_body(), 16 * 1024)
            .await
            .unwrap();
        let body = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(body.contains("cpu"));
        assert!(body.contains("41.5"));
    }

    #[tokio::test]
    async fn current_returns_latest_reading_per_sensor_from_store() {
        let store = Arc::new(SqliteStore::in_memory().unwrap());
        let now = Utc::now();
        store
            .insert_readings(&[
                SensorReading {
                    sensor_name: "nvidia:0:RTX".to_string(),
                    temperature_c: 52.0,
                    recorded_at: now - Duration::minutes(2),
                },
                SensorReading {
                    sensor_name: "nvidia:0:RTX".to_string(),
                    temperature_c: 55.0,
                    recorded_at: now - Duration::minutes(1),
                },
                SensorReading {
                    sensor_name: "hwmon:k10temp:Tctl".to_string(),
                    temperature_c: 48.0,
                    recorded_at: now,
                },
            ])
            .unwrap();

        let state = WebState {
            store,
            collector: Arc::new(MockCollector::failing()),
        };
        let app = router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/current")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), 32 * 1024).await.unwrap();
        let body = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(body.contains("hwmon:k10temp:Tctl"));
        assert!(body.contains("nvidia:0:RTX"));
        assert!(body.contains("55.0"));
        assert!(!body.contains("52.0"));
    }
}
