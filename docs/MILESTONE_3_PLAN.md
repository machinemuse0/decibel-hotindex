# Milestone 3 Tracking Plan

Milestone 3 adds the Decibel parser adapter path without making local development depend on live Aptos gRPC. The first pass is fixture-first: raw JSONL fixture records are normalized into the same dataset layout that later bounded mainnet recordings will use.

## Scope

Included:

- `decibel-dataset fixture`
- `decibel-dataset normalize --format fixture-jsonl`
- Decibel package/orderbook filtering
- Trade/order/position/liquidation/unknown event routing
- Unknown event payload preservation
- Manifest parser metadata and raw/normalized/query sha256 coverage
- Offline fixture pipeline smoke: `fixture -> normalize -> build-query-corpus -> replay(memory)`

Deferred:

- Live Aptos Hosted Transaction Stream implementation
- Aptos Transaction protobuf decoding
- zstd raw chunk writer
- Full parity with every Decibel event type from the upstream example

## M3-01: Fixture Raw Dataset Command

Status: Completed

Command:

```bash
rtk cargo run -p decibel-dataset -- fixture --out /private/tmp/decibel-hotindex-m3-smoke --events 20
```

Acceptance:

- Writes `raw/fixture_events.jsonl`.
- Writes a fixture checkpoint without calling Aptos gRPC.
- Uses configured or default Decibel package/orderbook addresses.

## M3-02: Parser Adapter

Status: Completed

Acceptance:

- Filters events by Decibel package/orderbook address.
- Maps `TradeEvent` into fills and builder attribution when fields exist.
- Maps order and position events into normalized rows.
- Maps liquidation/funding/market/builder-fee events into activity rows.
- Preserves unknown Decibel events in `unknown_events.ndjson`.

## M3-03: Normalize Fixture JSONL

Status: Completed

Command:

```bash
rtk cargo run -p decibel-dataset -- normalize --input /private/tmp/decibel-hotindex-m3-smoke/raw/fixture_events.jsonl --out-dir /private/tmp/decibel-hotindex-m3-smoke/normalized --format fixture-jsonl --parser-commit fixture-local
```

Acceptance:

- Writes normalized tx/event/fill/order/position/builder/activity/unknown files.
- Writes `parse_warnings.log`.
- Writes `manifest.json` with parser source, parser commit, version range, counts, and sha256 entries.

## M3-04: Offline Pipeline Smoke

Status: Completed

Commands:

```bash
rtk cargo run -p decibel-dataset -- build-query-corpus --events /private/tmp/decibel-hotindex-m3-smoke/normalized/events.ndjson --out-dir /private/tmp/decibel-hotindex-m3-smoke/queries --seed 42
rtk cargo run -p decibel-dataset -- replay --dataset /private/tmp/decibel-hotindex-m3-smoke --engine memory
```

Acceptance:

- Query corpus uses normalized fixture rows.
- Manifest query hashes are updated.
- Replay validates manifest hashes before loading rows.
- Memory replay prints stats and checksums.

## Open M3 Blocker

Live mainnet recording remains blocked on the Aptos Transaction Stream auth/protobuf implementation. The fixture parser path keeps parser/storage/API/benchmark work moving while preserving the same dataset contract that real recordings will use.
