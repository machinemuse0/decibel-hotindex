# Spikes

Last updated: 2026-05-27

This document tracks early dependency and data-source probes. These spikes are intentionally separate from mainline implementation so missing credentials or bindings do not block Milestone 0 + 1.

## S1: rust-toplingdb Availability

Status: Ready to run after recorder gRPC client is wired

Objective: determine whether the local environment can compile, link, and run a minimal ToplingDB Rust binding smoke test.

Why this matters:

- ToplingDB is core to the final comparison.
- The MVP must not block on unavailable bindings.
- The result decides whether Milestone 4 implements a real backend or an explicit `toplingsdb` feature-gated stub.

Inputs:

- Local rust-toplingdb checkout or registry dependency, if available.
- Existing ToplingDB native libraries, if required.

Command plan:

```bash
rtk cargo new /tmp/decibel-toplingdb-spike
rtk cargo check
rtk cargo run
```

The exact dependency and code snippet should be filled in when the binding source is confirmed.

Success criteria:

- A minimal program can open a temporary ToplingDB database.
- It can put/get one key.
- It can close cleanly.
- Build/link instructions are documented.

Failure criteria:

- Binding cannot be found.
- Native library cannot be linked.
- Basic put/get fails.
- Build requires undocumented system state.

Fallback:

- Keep `toplingsdb` as a compile-time feature-gated stub.
- Implement RocksDB and MemoryEngine fully.
- Document ToplingDB enablement steps once binding is available.

Result:

```text
Not run yet.
```

## S2: Aptos Transaction Stream Auth and Bounded Recording

Status: Planned

Objective: verify that a bounded Aptos mainnet Transaction Stream range can be read with current auth requirements, without designing the full recorder first.

Why this matters:

- Dataset pipeline depends on getting one bounded real mainnet sample as early as possible.
- Benchmark runner must remain offline; online access belongs only in dataset recording.
- Auth/rate-limit issues must be discovered before parser/storage work assumes live data availability.

Inputs:

- `APTOS_GRPC_AUTH_TOKEN` exported locally from the Geomi/Aptos API key. Do not commit or log token material.
- Network: mainnet first; testnet only as fallback for parser/auth debugging.
- Decibel package/orderbook addresses from official Decibel indexer example.
- Small bounded version range.

Command plan:

```bash
rtk cargo run -p decibel-dataset -- record \
  --live \
  --network mainnet \
  --endpoint grpc.mainnet.aptoslabs.com:443 \
  --auth-token-env APTOS_GRPC_AUTH_TOKEN \
  --start-version 4365621793 \
  --end-version 4365622793 \
  --raw-format protobuf-zstd \
  --out-dir datasets/spikes/mainnet_raw_sample/raw
```

The Transaction Stream protobuf client is wired through Aptos `aptos-protos`; the remaining blocker is running the command with a local `APTOS_GRPC_AUTH_TOKEN`.

Success criteria:

- Auth succeeds.
- A small bounded mainnet range can be read.
- Raw protobuf/zstd records are written locally.
- Raw chunk sha256 is recorded.
- Resume/checkpoint requirements are clear.

Failure criteria:

- Token/API-key flow is unavailable.
- gRPC endpoint rejects request.
- Rate limits prevent even a small bounded range.
- Event format differs from parser assumptions.

Fallback:

- Keep Milestone 2 on synthetic fixture dataset.
- Use official example output or manually captured raw samples for parser tests.
- Mark real recording blocked but keep local replay/benchmark development moving.

Result:

```text
Passed locally.

recorded transaction stream: tx=2 range=4365621793..4365621794 chunk=/private/tmp/decibel-hotindex-live-smoke/raw/transactions_4365621793_4365621794.pb.zst

Raw chunk decoded as 2 length-delimited Aptos Transaction protobuf messages with first_version=4365621793, last_version=4365621794, remaining_bytes_after_limit=0.
```
