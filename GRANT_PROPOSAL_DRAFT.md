# Grant Proposal Draft

## Title

Decibel HotIndex: Reproducible Low-Latency Builder Analytics and Serving Layer for Decibel on Aptos

## Summary

Decibel HotIndex is an open-source local serving layer and builder analytics gateway for Decibel markets on Aptos. It records bounded mainnet transaction-stream data once into a local raw archive, normalizes Decibel events, materializes market/account/builder-code indexes, and benchmarks RocksDB and ToplingDB under the same schema, dataset, keyset, and workload.

## Problem

Decibel produces high-frequency on-chain trading data. Builders, bots, dashboards, market makers, and analytics tools need low-latency local query paths for market fills, account activity, builder-code attribution, and dashboard workloads. Generic indexers are not optimized for this Decibel-specific access pattern.

## Solution

The project provides:

- mainnet-first reproducible dataset pipeline
- immutable raw archive using protobuf + zstd
- Decibel-specific normalized event model
- backend-neutral `StorageEngine`
- RocksDB baseline and ToplingDB feature-gated backend
- checksum-based cross-backend equivalence
- REST APIs and benchmark reports

## Milestones

1. Workspace, dataset layout, core contracts, and MemoryEngine.
2. Mainnet dataset recording, normalization, query corpus, and replay.
3. RocksDB/ToplingDB backend comparison with checksum equivalence.
4. REST API, demo dashboard, benchmark report, and documentation.

## Non-Goals

- No trading strategy.
- No matching engine.
- No private-key handling.
- No official builder-code settlement claims.

## Budget

Requested budget: TBD

Timeline: 4-6 weeks for MVP, benchmark, and documentation.
