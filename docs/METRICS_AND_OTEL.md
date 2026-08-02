# Metrics store and OpenTelemetry

Product spec: `specs/07-metrics-telemetry.md`.

## Local SQLite store (v0.4)

- **Off by default.** Enable in Options → Metrics store.
- Database path: `%APPDATA%\fancontrol-rs\metrics.sqlite` (shown in Options).
- Sample interval and retention (days) are configurable.
- **Export CSV** writes under `%APPDATA%\fancontrol-rs\exports\metrics-<unix>.csv`.

Columns: `ts_ms,sensor_id,value,kind,unit`.

The control loop never waits on SQLite: samples go through a bounded channel to a background thread.

## Graph multi-kind

Options → Graph sensors lists:

1. Temperatures
2. GPU and other metrics (power, util, clocks, VRAM, fan %)

If you select two different units (for example °C and W), the UI stacks two plots (shared time window). More than two units: only the first two unit groups are drawn.

## OpenTelemetry (v0.4.0: settings only)

- Options can save **endpoint** and an enable flag.
- **OTLP export is not wired yet** in v0.4.0; prefer **OTLP/HTTP** to a collector you run when it lands (e.g. Grafana Alloy or OpenTelemetry Collector on `http://127.0.0.1:4318`).
- No project-operated cloud. Metrics leave the machine only if you enable export and point at an endpoint you control.

### Suggested local collector (when OTEL export ships)

1. Run Grafana Alloy or `otelcol` with an OTLP HTTP receiver on port 4318.
2. Enable OTEL in Options and set the endpoint.
3. Confirm gauges appear in your backend (Prometheus/Grafana optional).

## Security

See `docs/SECURITY.md`: SQLite under the user profile; HTTP only when OTEL is explicitly enabled.
