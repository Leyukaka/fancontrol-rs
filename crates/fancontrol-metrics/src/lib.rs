//! Metric sinks for fancontrol-rs: local SQLite, CSV export, multi-sink fan-out.
//!
//! OpenTelemetry export lands after the local store path is stable
//! (`specs/07-metrics-telemetry.md`).

mod multi;
mod sqlite;

pub use multi::{MultiSink, NullSink};
pub use sqlite::{default_metrics_db_path, CsvExportOptions, SqliteMetricsStore, SqliteStoreConfig};

use fancontrol_core::MetricSample;

/// Best-effort consumer of metric batches. Must never panic into the UI/poll path.
pub trait MetricSink: Send {
    fn record(&mut self, batch: &[MetricSample]);
}
