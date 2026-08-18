# Metrics, local store, and OpenTelemetry

Spec for **v0.4.0** metrics pipeline. Complements UI graph / GPU panel (`04-ui.md`, host sensors).

## Goals

1. **Graph multi-kind series** (not only temperatures): GPU power, util, clocks, VRAM, fan %, **CPU package power**, **CPU power limit / TDP**, **DRAM power** when available, etc.
2. **Local metrics store** (SQLite), opt-in, configurable retention and sample interval, **manual CSV export**.
3. **OpenTelemetry metrics export** (opt-in), protocol chosen after store lands (default target: **OTLP/HTTP** to a user-run local collector).

## Non-goals (v0.4)

- Cloud backend operated by the project
- Bundling Grafana / Alloy / otelcol in the installer
- OTEL traces/logs (metrics only)
- Replaying full SQLite history into the live graph (may come later)
- Blocking the PWM control loop on I/O or network

## Architecture

```
Poll snapshot
    → MetricSample[]
    → MultiSink
         ├── (implicit) UI histories / GPU panel from same poll values
         ├── LocalSqliteSink   (opt-in)
         └── OtlpSink          (opt-in, OTLP/HTTP JSON)
```

### MetricSample (domain)

| Field | Type | Notes |
|-------|------|--------|
| `sensor_id` | string | Stable id (`host.gpu0.power.draw`, `host.cpu.power.package`, `host.ram.power`, `pawnio.0.temp.CPUTIN`, …) |
| `label` | string | Display name at sample time |
| `kind` | `SensorKind` | Temperature, Power, Load, … |
| `unit` | optional string | `°C`, `W`, `%`, `MHz`, `MiB` |
| `value` | f64 | Numeric reading |
| `ts_ms` | i64 | Unix epoch milliseconds |

### MetricSink

```text
trait MetricSink: Send {
    fn record(&mut self, batch: &[MetricSample]);
    // best-effort; errors logged, never panic into UI/poll
}
```

`MultiSink` fans out to enabled sinks. Sinks that do disk/network work use a **background channel/thread**.

## Graph multi-kind

- Options picker sections: **Temperatures** and **GPU / other metrics** (includes CPU/RAM power).
- Live series use the same ring buffer concept as today’s thermal history (`SampleHistory`).
- **Y axes**:
  - One unit among selected series → single Y axis.
  - Two units → dual Y (left primary, right secondary).
  - More than two units selected → keep first two unit groups; warn in UI (no third axis).
- Shader graph styles stay **temperature-driven** (non-temp series do not drive heat uniforms).
- **Power (W) Y-axis**: ceiling from `max(live GPU power.limit, host.cpu.power.limit)` when available (readable scale at idle), not a fixed ~1000 W range.
- **Default graph seed** (first run): CPU-like temp + GPU temp when known **+** `host.cpu.power.package` (and `host.ram.power` / CPU limit when present).

## CPU / DRAM package power (PawnIO MSR)

**Backend rule:** PawnIO modules only - no WinRing0, no WMI/PowerShell, no EMI fallback in v1.

| Sensor id | Meaning | Source (read-only) |
|-----------|---------|---------------------|
| `host.cpu.power.package` | Package power (W) | ΔE/Δt from package energy MSR |
| `host.cpu.power.limit` | TDP / package power info (W) | Power-info MSR when available |
| `host.ram.power` | DRAM power (W) | DRAM energy domain when available |

**Vendor order:** try **AMD** (`AMDFamily17` - Zen fam 17h-1Ah) then **Intel** (`IntelMSR` RAPL).

| Domain | AMD | Intel |
|--------|-----|--------|
| Package W | yes | yes |
| Power limit / TDP-ish | no (omit sensor) | `MSR_PKG_POWER_INFO` when valid |
| DRAM W | no | `MSR_DRAM_ENERGY_STATUS` when readable |

- Requires elevated PawnIO session (same as Super I/O).
- Monitoring only - **no** write of PL1/PL2 or undervolt MSRs.
- If a domain is missing on a CPU, omit the sensor (do not invent values).
- Fan **curves still use CPU-like temperatures only** - power series are graph/metrics only.

## Local SQLite store

| Setting | Default | Notes |
|---------|---------|--------|
| `metrics_store_enabled` | **false** | Opt-in |
| `metrics_sample_secs` | 5 | Write cadence (independent of graph sample if desired) |
| `metrics_retention_days` | 7 | Purge older rows |
| path | `%APPDATA%/fancontrol-rs/metrics.sqlite` | Show in Options; open config folder |

### Schema

```sql
CREATE TABLE samples (
  ts_ms INTEGER NOT NULL,
  sensor_id TEXT NOT NULL,
  value REAL NOT NULL,
  kind TEXT NOT NULL,
  unit TEXT,
  PRIMARY KEY (ts_ms, sensor_id)
);
CREATE INDEX idx_samples_id_ts ON samples(sensor_id, ts_ms);
```

### CSV export

- Manual button in Options (only when store enabled or DB exists).
- Export selected range or full retained window → user-chosen path or under config dir `exports/metrics-YYYYMMDD-HHMMSS.csv`.
- Header: `ts_ms,sensor_id,value,kind,unit`

## OpenTelemetry

- Disabled by default.
- User provides endpoint (e.g. `http://127.0.0.1:4318`).
- Export gauges under a stable metric name family (e.g. `fancontrol.sensor` with attributes `sensor_id`, `kind`, `unit`).
- **Protocol**: **OTLP/HTTP JSON** (`POST /v1/metrics`). gRPC is out of scope.
- Failure mode: log and backoff; no modal spam.

Document smoke setup in `docs/METRICS_AND_OTEL.md` (collector → Prometheus/Grafana optional).

## Security / privacy

- No metrics leave the machine unless the user enables OTEL and points at an endpoint they control.
- Local DB is plain SQLite under the user profile (same trust boundary as profiles JSON).
- See `docs/SECURITY.md` for network note when OTEL is enabled.

## Crate layout (target)

| Piece | Crate |
|-------|--------|
| `MetricSample`, `SensorKind` reuse | `fancontrol-core` |
| Sinks (SQLite, CSV export, OTLP/HTTP) | `fancontrol-metrics` |
| Wire poll → sinks; Options | `fancontrol-ui` |

## Acceptance (v0.4.0)

- [x] Graph can plot GPU power / util with dual unit plots vs temps.
- [x] Power (W) axis uses GPU power.limit when available.
- [x] Store off by default; enable → SQLite grows; retention purges; CSV export works.
- [x] Start with Windows opt-in (Options + first-run prompt).
- [x] OTEL off by default; enable + local collector → `fancontrol.sensor` gauges over OTLP/HTTP.
- [x] CI green; no regression on PWM / GPU panel.

## Acceptance (CPU / DRAM power)

- [x] AMD package power via PawnIO `AMDFamily17` → `host.cpu.power.package` plottable + metrics store.
- [x] Intel RAPL via `IntelMSR`: package + limit + DRAM when present.
- [x] Power Y ceiling considers CPU limit as well as GPU `power.limit`.
- [x] Default graph seed includes package power when the sensor exists.
- [x] Read-only MSRs only; graceful omit when PawnIO/module/CPU unsupported.
