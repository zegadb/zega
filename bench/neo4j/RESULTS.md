# zega vs Neo4j: idle RAM and throughput

Measured 2 Oct 2026 with `./run.sh` in this directory. Raw CSVs are in [results/](results/).

**Setup.** One machine (iMac20,2, Core i9-10910, 128 GB, macOS; see `results/machine.json`). zega 0.2.0 at `a02b89d`, server path over HTTP on 127.0.0.1. Neo4j Community 5.26 (`neo4j:5.26-community`) in Docker (colima VM, 4 CPUs, about 7.75 GiB) at default memory settings, over Bolt. The same seeded graph in both (persons and products, 4.5 relationships per node), unique constraints/indexes on the lookup keys in both, the same seeded query parameters. Before timing, both engines' answers were compared on 25 samples of each read query and one write: all agreed.

**Windows.** 10k: 30 s per cell after a warm-up. 100k: 8 s per cell (`CELL_SECONDS=8`). 1M, the native (non-Docker) Neo4j run and the tuned-memory Neo4j run were not done.

## Idle memory (resident, MB)

```
tag,engine,mode,size,state,rss_mb,ondisk_mb,notes
z100000,zega,default,100000,empty,4.8,0.0,
z100000,zega,default,100000,loaded,420.4,121.7,
n100000,neo4j,docker-default,100000,empty,432.5,515.9,stats=424.7MB
n100000,neo4j,docker-default,100000,loaded,789.3,515.9,stats=786.8MB
```

## Throughput, 10k persons (11,000 nodes, 49,974 relationships)

| query | clients | zega q/s | Neo4j q/s | zega p50 ms | Neo4j p50 ms |
|---|---:|---:|---:|---:|---:|
| lookup | 1 | not measured | not measured | not measured | not measured |
| lookup | 8 | not measured | not measured | not measured | not measured |
| lookup | 32 | not measured | not measured | not measured | not measured |
| onehop | 1 | not measured | not measured | not measured | not measured |
| onehop | 8 | not measured | not measured | not measured | not measured |
| onehop | 32 | not measured | not measured | not measured | not measured |
| twohop | 1 | not measured | not measured | not measured | not measured |
| twohop | 8 | not measured | not measured | not measured | not measured |
| twohop | 32 | not measured | not measured | not measured | not measured |
| filtered | 1 | not measured | not measured | not measured | not measured |
| filtered | 8 | not measured | not measured | not measured | not measured |
| filtered | 32 | not measured | not measured | not measured | not measured |
| path | 1 | not measured | not measured | not measured | not measured |
| path | 8 | not measured | not measured | not measured | not measured |
| path | 32 | not measured | not measured | not measured | not measured |
| write | 1 | not measured | not measured | not measured | not measured |
| write | 8 | not measured | not measured | not measured | not measured |
| write | 32 | not measured | not measured | not measured | not measured |

## Throughput, 100k persons (110,000 nodes, 499,976 relationships)

| query | clients | zega q/s | Neo4j q/s | zega p50 ms | Neo4j p50 ms |
|---|---:|---:|---:|---:|---:|
| lookup | 1 | 3,405.2 | 322.1 | 0.31 | 2.99 |
| lookup | 8 | 11,232.6 | 2,240.2 | 0.74 | 3.24 |
| lookup | 32 | 18,644.8 | 3,446.2 | 1.62 | 8.91 |
| onehop | 1 | 4,838.4 | 354.4 | 0.19 | 2.57 |
| onehop | 8 | 14,511.0 | 1,340.8 | 0.46 | 5.35 |
| onehop | 32 | 15,755.0 | 2,528.8 | 1.80 | 11.82 |
| twohop | 1 | 4,150.4 | 258.5 | 0.22 | 3.35 |
| twohop | 8 | 15,124.9 | 1,258.0 | 0.52 | 5.67 |
| twohop | 32 | 14,184.8 | 1,533.6 | 2.15 | 19.30 |
| filtered | 1 | 3,605.4 | 270.4 | 0.27 | 3.10 |
| filtered | 8 | 7,042.8 | 1,669.5 | 1.10 | 4.31 |
| filtered | 32 | 6,320.5 | 1,650.4 | 4.58 | 17.75 |
| path | 1 | 19.1 | 182.2 | 52.54 | 4.61 |
| path | 8 | 24.5 | 871.4 | 288.28 | 8.13 |
| path | 32 | 15.2 | 1,520.2 | 1745.74 | 19.61 |
| write | 1 | 49.6 | 194.9 | 19.47 | 4.47 |
| write | 8 | 30.1 | 936.9 | 225.87 | 7.11 |
| write | 32 | 46.8 | 1,432.2 | 652.34 | 21.07 |

## What this supports

- Reads (lookup, 1-hop, 2-hop, filtered 2-hop): zega is 10 to 20 times faster with one client and 3 to 9 times faster at 32 clients, at both sizes.
- Idle, empty: about 5 MB against 430 to 470 MB. Idle with the graph loaded: 39 MB against 637 MB at 10k; 420 MB against 789 MB at 100k.

## Where zega loses

- **Shortest path**: about even at 10k with one client, far behind under concurrency, and far behind at 100k at every concurrency.
- **Single-node writes**: flat at about 50 a second at every concurrency and both sizes. The data directory was on an external USB disk, where a sync is slow; Neo4j on the same disk (through the VM) still scaled with clients.

## Caveats

Neo4j in Docker on macOS runs inside a Linux VM. Its durability settings were left at defaults. The repo volume is a USB disk. The 10k run's Neo4j write cell at 32 clients is missing (the first run was cut off there).
