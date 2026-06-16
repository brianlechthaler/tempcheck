use std::sync::Arc;
use std::time::Duration;

use tokio::sync::watch;
use tokio::time;
use tracing::{error, info, warn};

use crate::collector::TemperatureCollector;
use crate::storage::TemperatureStore;

pub async fn run_daemon<C, S>(
    collector: Arc<C>,
    store: Arc<S>,
    interval: Duration,
    mut shutdown: watch::Receiver<bool>,
) where
    C: TemperatureCollector + 'static,
    S: TemperatureStore + 'static,
{
    info!(?interval, "temperature daemon started");
    let mut ticker = time::interval(interval);
    ticker.set_missed_tick_behavior(time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                match collector.collect() {
                    Ok(readings) if readings.is_empty() => {
                        warn!("no thermal sensors found");
                    }
                    Ok(readings) => {
                        match store.insert_readings(&readings) {
                            Ok(n) => info!(count = n, "stored temperature readings"),
                            Err(e) => error!(error = %e, "failed to store readings"),
                        }
                    }
                    Err(e) => error!(error = %e, "failed to collect temperatures"),
                }
            }
            changed = shutdown.changed() => {
                if changed.is_ok() && *shutdown.borrow() {
                    info!("temperature daemon shutting down");
                    break;
                }
            }
        }
    }
}

pub fn shutdown_channel() -> (watch::Sender<bool>, watch::Receiver<bool>) {
    watch::channel(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    use crate::collector::{MockCollector, SensorReading};

    #[tokio::test]
    async fn daemon_stores_on_tick_and_stops_on_shutdown() {
        let reading = SensorReading {
            sensor_name: "cpu".to_string(),
            temperature_c: 42.0,
            recorded_at: Utc::now(),
        };
        let collector = Arc::new(MockCollector::new(vec![reading]));
        let store = Arc::new(crate::storage::SqliteStore::in_memory().unwrap());
        let (tx, rx) = shutdown_channel();

        let handle = tokio::spawn(run_daemon(
            collector,
            Arc::clone(&store),
            Duration::from_millis(50),
            rx,
        ));

        time::sleep(Duration::from_millis(120)).await;
        tx.send(true).unwrap();
        handle.await.unwrap();

        let rows = store.recent_readings(None, 10).unwrap();
        assert!(!rows.is_empty());
    }
}
