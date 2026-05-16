# ADR 0001 — Telemetry and Remote Experiments Omission

**Status:** Accepted  
**Date:** 2025-05-16  
**Deciders:** core maintainers  
**Ticket:** todo `remaining-telemetry-experiments`

---

## Context

The upstream TypeScript codebase contains several analytics and experiment-evaluation
subsystems that were candidates for porting during the Rust rewrite:

| Upstream module | Role |
|---|---|
| `services/analytics/datadog.ts` | Datadog metrics/trace sink |
| `services/analytics/firstPartyEventLogger.ts` | 1P event emission pipeline |
| `services/analytics/firstPartyEventLoggingExporter.ts` | OTLP/gRPC exporter |
| `services/analytics/sink.ts` / `sinkKillswitch.ts` | Sink routing and circuit-breaker |
| `services/analytics/growthbook.ts` | GrowthBook remote feature-flag evaluation |
| `types/…/growthbook_experiment_event.ts` | Generated Protobuf type for experiment events |
| `utils/telemetry/*` | BigQuery exporter, Perfetto tracing, session/skill events, OTEL instrumentation |
| `utils/fileOperationAnalytics.ts` | Per-operation analytics counter |
| `utils/plugins/fetchTelemetry.ts` | Plugin telemetry fetch helper |

Porting any of these components faithfully would require:

1. **External service dependencies** — Datadog agent, GrowthBook SaaS / self-hosted,
   BigQuery project, or a first-party event ingestion endpoint.  None of these
   services are available or appropriate in a local-first, offline-capable CLI tool.
2. **Privacy regression** — Silently phoning home to analytics backends would
   contradict the local-first positioning of this port and would require user
   consent UI, GDPR/CCPA handling, and audit controls that are out of scope.
3. **Binary bloat** — The OTEL SDK, Datadog client, and GrowthBook SDK together
   add hundreds of kilobytes plus transitive C library dependencies.

## Decision

**We will not port any analytics emission, telemetry export, or GrowthBook remote
experiment evaluation into this Rust codebase.**

Specifically:

- **No Datadog sink.** `services/analytics/datadog.ts` and the associated
  `sink.ts`/`sinkKillswitch.ts` have no Rust equivalent.  There is no metrics
  or trace emission from this binary.
- **No first-party event pipeline.** `firstPartyEventLogger.ts` and
  `firstPartyEventLoggingExporter.ts` have no Rust equivalent.  Usage events
  are not collected or forwarded.
- **No GrowthBook remote evaluation.** `services/analytics/growthbook.ts` and
  the associated generated types have no Rust equivalent.  All feature gates are
  resolved **statically** at startup from a [`FeatureSet`] value; no network
  request is made to determine which features are enabled.
- **No BigQuery / Perfetto / OTEL instrumentation** from `utils/telemetry/`.
  Internal developer tooling (Perfetto traces, BigQuery session data) is not
  appropriate for a local end-user CLI.
- **No per-operation analytics counters** (`fileOperationAnalytics.ts`,
  `fetchTelemetry.ts`).

## Surfacing the decision honestly

`SyncStatusReport` (in `wonder-of-u-storage`) contains two fields that are
**permanently `Unsupported`** with explicit reason strings:

```rust
pub analytics: RemoteSurfaceStatus,   // "no Datadog/first-party event sink"
pub experiments: RemoteSurfaceStatus, // "GrowthBook remote evaluation not implemented"
```

The `/status` command emits these values as `analytics=unsupported` and
`experiments=unsupported` so operators can see the posture at a glance.

Unit tests in `wonder-of-u-storage` (see
`sync_status_report_analytics_and_experiments_are_unsupported`) and integration
tests in `wonder-of-u-cli` (see `status_output_includes_analytics_and_experiments_unsupported`)
act as regression guards — a future contributor cannot silently change these
surfaces to `ok` without breaking the test suite.

## Consequences

* **Positive:** No privacy-sensitive data leaves the machine.  No external service
  credentials are required to build or run the binary.  Binary size is smaller.
* **Positive:** The decision is explicit and auditable — parity-ledger entries for
  all omitted modules are marked `intentionally-omitted/local-first` with a
  reference to this ADR.
* **Negative:** Operators who need usage analytics must instrument the binary
  themselves (e.g., wrap invocations with their own telemetry agent).
* **Deferred:** If a future release targets a managed cloud deployment where
  opt-in telemetry is appropriate, a new ADR should be written, a `Telemetry`
  feature flag added to `FeatureSet`, and the `SyncStatusReport` surfaces updated
  to reflect the new posture.  The permanently-unsupported tests should be removed
  or replaced at that point.

## Alternatives considered

| Alternative | Why rejected |
|---|---|
| Opt-in telemetry flag at build time (feature flag) | Adds complexity now for a use-case with no current consumer; deferred per above |
| Stub types that compile but are no-ops | Would give a false impression of parity; dishonest to the operator |
| Linking GrowthBook Rust SDK against a local static config | GrowthBook SDK has no stable Rust port; JSON config could be used instead, but static `FeatureSet` already covers this need |
