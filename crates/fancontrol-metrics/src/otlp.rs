//! Opt-in OTLP/HTTP metrics export (JSON). Background sender, never blocks poll/UI.

use crate::MetricSink;
use fancontrol_core::MetricSample;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::thread;
use std::time::{Duration, Instant};

enum OtlpCmd {
    Batch(Vec<MetricSample>),
    Shutdown,
}

/// Non-blocking OTLP/HTTP sink. `record` drops the batch if the channel is full.
pub struct OtlpSink {
    tx: SyncSender<OtlpCmd>,
}

impl OtlpSink {
    /// Spawn a worker. `endpoint` is the collector base (`http://127.0.0.1:4318`)
    /// or a full `/v1/metrics` URL.
    pub fn spawn(endpoint: impl Into<String>) -> Option<Self> {
        let url = export_url(&endpoint.into());
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            tracing::warn!(%url, "otel endpoint must be http(s); export disabled");
            return None;
        }
        let (tx, rx) = mpsc::sync_channel::<OtlpCmd>(32);
        thread::Builder::new()
            .name("metrics-otlp".into())
            .spawn(move || worker_loop(url, rx))
            .ok()?;
        Some(Self { tx })
    }
}

impl MetricSink for OtlpSink {
    fn record(&mut self, batch: &[MetricSample]) {
        if batch.is_empty() {
            return;
        }
        if self.tx.try_send(OtlpCmd::Batch(batch.to_vec())).is_err() {
            tracing::debug!("metrics otlp channel full; dropping batch");
        }
    }
}

impl Drop for OtlpSink {
    fn drop(&mut self) {
        let _ = self.tx.try_send(OtlpCmd::Shutdown);
    }
}

pub(crate) fn export_url(endpoint: &str) -> String {
    let trimmed = endpoint.trim().trim_end_matches('/');
    if trimmed.ends_with("/v1/metrics") {
        trimmed.to_string()
    } else {
        format!("{trimmed}/v1/metrics")
    }
}

pub(crate) fn build_otlp_json(batch: &[MetricSample]) -> String {
    let points: Vec<serde_json::Value> = batch
        .iter()
        .map(|s| {
            let nanos = s.ts_ms.saturating_mul(1_000_000).max(0);
            let mut attrs = vec![kv("sensor_id", &s.sensor_id), kv("kind", s.kind.as_str())];
            if let Some(unit) = &s.unit {
                attrs.push(kv("unit", unit));
            }
            if !s.label.is_empty() {
                attrs.push(kv("label", &s.label));
            }
            serde_json::json!({
                "asDouble": s.value,
                "timeUnixNano": nanos.to_string(),
                "attributes": attrs,
            })
        })
        .collect();

    serde_json::json!({
        "resourceMetrics": [{
            "resource": {
                "attributes": [
                    kv("service.name", "fancontrol-rs"),
                    kv("service.version", env!("CARGO_PKG_VERSION")),
                ]
            },
            "scopeMetrics": [{
                "scope": { "name": "fancontrol-rs", "version": env!("CARGO_PKG_VERSION") },
                "metrics": [{
                    "name": "fancontrol.sensor",
                    "unit": "1",
                    "gauge": { "dataPoints": points }
                }]
            }]
        }]
    })
    .to_string()
}

fn kv(key: &str, value: &str) -> serde_json::Value {
    serde_json::json!({
        "key": key,
        "value": { "stringValue": value }
    })
}

fn worker_loop(url: String, rx: Receiver<OtlpCmd>) {
    let mut backoff = Duration::from_secs(2);
    let mut silenced_until = Instant::now();
    while let Ok(cmd) = rx.recv() {
        match cmd {
            OtlpCmd::Shutdown => break,
            OtlpCmd::Batch(batch) => {
                if Instant::now() < silenced_until {
                    continue;
                }
                match post_batch(&url, &batch) {
                    Ok(()) => {
                        backoff = Duration::from_secs(2);
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, %url, "otel export failed; backing off");
                        silenced_until = Instant::now() + backoff;
                        backoff = (backoff * 2).min(Duration::from_secs(60));
                    }
                }
            }
        }
    }
}

fn post_batch(url: &str, batch: &[MetricSample]) -> Result<(), String> {
    let body = build_otlp_json(batch);
    let mut resp = ureq::post(url)
        .header("Content-Type", "application/json")
        .header("User-Agent", "fancontrol-rs-otlp")
        .send(&body)
        .map_err(|e| format!("request failed: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        let text = resp.body_mut().read_to_string().unwrap_or_default();
        return Err(format!("HTTP {status}: {text}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use fancontrol_core::SensorKind;

    #[test]
    fn export_url_appends_path() {
        assert_eq!(
            export_url("http://127.0.0.1:4318"),
            "http://127.0.0.1:4318/v1/metrics"
        );
        assert_eq!(
            export_url("http://127.0.0.1:4318/v1/metrics/"),
            "http://127.0.0.1:4318/v1/metrics"
        );
    }

    #[test]
    fn payload_includes_sensor_and_kind() {
        let batch = [MetricSample::new(
            "host.dimm0.temp",
            "DIMM 0 Temp",
            SensorKind::Temperature,
            Some("°C".into()),
            35.8,
            1_700_000_000_000,
        )];
        let json = build_otlp_json(&batch);
        assert!(json.contains("fancontrol.sensor"));
        assert!(json.contains("host.dimm0.temp"));
        assert!(json.contains("temperature"));
        assert!(json.contains("DIMM 0 Temp"));
        assert!(!json.contains("experimental"));
    }
}
