//! Fan-out and no-op sinks.

use crate::MetricSink;
use fancontrol_core::MetricSample;

/// Discards all samples (default when store/OTEL off).
#[derive(Debug, Default)]
pub struct NullSink;

impl MetricSink for NullSink {
    fn record(&mut self, _batch: &[MetricSample]) {}
}

/// Forwards each batch to every enabled child sink.
pub struct MultiSink {
    sinks: Vec<Box<dyn MetricSink>>,
}

impl Default for MultiSink {
    fn default() -> Self {
        Self::new()
    }
}

impl MultiSink {
    pub fn new() -> Self {
        Self { sinks: Vec::new() }
    }

    pub fn push(&mut self, sink: Box<dyn MetricSink>) {
        self.sinks.push(sink);
    }

    pub fn is_empty(&self) -> bool {
        self.sinks.is_empty()
    }
}

impl MetricSink for MultiSink {
    fn record(&mut self, batch: &[MetricSample]) {
        if batch.is_empty() {
            return;
        }
        for s in &mut self.sinks {
            s.record(batch);
        }
    }
}
