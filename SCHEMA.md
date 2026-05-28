# Schema

This document captures the initial logical schema. Persistent backend details will be implemented after the MemoryEngine contract is stable.

## Logical Column Families

```text
cf_tx_by_version
cf_raw_event_by_version_idx
cf_event_by_type_version
cf_market_recent_activity
cf_fills_by_market_time
cf_fills_by_account_time
cf_orders_by_account_time
cf_order_by_id
cf_positions_by_account_market
cf_liquidations_by_market_time
cf_builder_code_fills
cf_builder_code_volume_window
cf_builder_code_accounts
cf_ingest_checkpoint
```

## Key Rules

- Integers use big-endian encoding.
- Recent-first scans use `reverse_ts_us = u64::MAX - timestamp_us`.
- Aptos addresses are normalized to lowercase `0x` + 64 hex chars.
- Backend implementations must use the same logical schema.

## MVP Query Paths

- transaction by version
- multi-get transaction versions
- market recent fills
- account recent fills
- builder-code fills
- builder-code volume
- ingest checkpoint
- storage checksum

## Semantics

Latest position/activity means latest observed within the indexed dataset range. Builder-code fee metrics are analytics estimates from parsed events, not official settlement statements.
