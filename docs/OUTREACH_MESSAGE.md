# Outreach Message

Subject: Decibel HotIndex bounded benchmark package for review

Hi <name>,

We are preparing Decibel HotIndex as a local serving and builder analytics layer
for Decibel events on Aptos. The benchmark package is designed around a
bounded, reproducible dataset rather than live RPC calls during measurement.

The review package includes:

- immutable Aptos Transaction Stream raw chunks with a dataset manifest
- normalized Decibel rows and deterministic query corpora
- RocksDB baseline materialization from the clean `main` worktree
- ToplingDB materialization from the isolated `topingdb` worktree
- per-logical-CF checksum comparison before any benchmark number is discussed
- benchmark reports that use the same schema, dataset, keyset, and workload

The important claim is intentionally narrow:

```text
same schema, same dataset, same keyset, same workload, checksum-passed
```

We are not claiming generic database superiority or official Decibel settlement
truth. Builder-code metrics are analytics estimates derived from parsed Decibel
events, and every report carries the dataset id and checksum status needed for
review.

The next useful feedback would be on:

- whether the bounded range contains the Decibel event mix you care about
- whether the query corpus matches dashboard/operator workflows
- whether the checksum and benchmark report format is sufficient for independent
  reproduction

Thanks,
<sender>
