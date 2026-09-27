These two fixtures were generated with the phase-1 implementation at
`734807302d7e88fc3daabeaea22e3225843e9a75` (origin/main, APS 24 phase 1).

Schema: `type T { n: <Int> }`.
Insert `n = 10` at 2024-01-01; update to `20` at 2024-02-01.
`history.graph` is a `.graph` export; `snapshot.bin` is `snapshot_bytes()`.
Both contain HIST payload version 1. Do not regenerate them with phase 2;
they prove compatibility with bytes actually emitted by the older writer.
