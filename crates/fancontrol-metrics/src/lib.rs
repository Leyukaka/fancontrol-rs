//! Metric sinks for fancontrol-rs: local SQLite, CSV export, OTLP/HTTP.

mod multi;
mod otlp;
mod sqlite;

pub use multi::{MultiSink, NullSink};
pub use otlp::OtlpSink;
pub use sqlite::{
    CsvExportOptions, SqliteMetricsStore, SqliteStoreConfig, default_metrics_db_path,
};

use fancontrol_core::MetricSample;

/// Best-effort consumer of metric batches. Must never panic into the UI/poll path.
pub trait MetricSink: Send {
    fn record(&mut self, batch: &[MetricSample]);
}
