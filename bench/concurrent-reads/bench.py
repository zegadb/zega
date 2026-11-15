#!/usr/bin/env python3
"""Concurrent `*path` reads against a running zega-server (zegadb/zega#147).

  python3 -I bench.py <url> <token-file> <out.json> [--mixed SECONDS [--mixed-only]]

The graph is an osm2zega road graph (the routing plan's Alberta or Calgary
graph): `Junction { osm at d w b }` joined by `drive` / `walk` / `bike` roads
with a `cost`. The route is the one the routing report timed for concurrency,
Tuscany (NW Calgary) to Mahogany (SE Calgary) by car, A* (`toward at`).

1. Clients: 1, 4 and 8 clients each send 8 routes back to back (after one
   warm-up route). Reported: routes/s (all routes over wall time) and the
   p50 / p95 of each route's wall time over HTTP. Every answer's cost is
   checked against the first one.
2. --mixed SECONDS: 4 route clients run for SECONDS while one writer creates a
   probe junction (no roads, so routes are unchanged) as fast as it is
   answered. After each answer the writer reads its probe back. At the end
   the number of probes must equal the writes answered, and every route
   during the run must have the same cost. Reported: routes/s and p50 / p95
   for reads, writes/s and p50 / p95 for writes.

Writes go into the data directory: run --mixed on a copy of the graph.
"""
import concurrent.futures as cf
import json
import statistics
import sys
import threading
import time
import urllib.error
import urllib.request

URL, TOKEN, OUT = sys.argv[1], open(sys.argv[2]).read().strip(), sys.argv[3]
MIXED = int(sys.argv[sys.argv.index("--mixed") + 1]) if "--mixed" in sys.argv else 0
FROM, TO = (51.1270, -114.2420), (50.8990, -113.9330)
PROBE_BASE = -1_000_000_000


def call(method, body, timeout=300):
    req = urllib.request.Request(URL + "/zql", method=method, data=json.dumps(body).encode(),
                                 headers={"authorization": "Bearer " + TOKEN, "content-type": "application/json",
                                          "accept": "application/json"})
    started = time.perf_counter()
    try:
        with urllib.request.urlopen(req, timeout=timeout) as response:
            answer = json.loads(response.read())
    except urllib.error.HTTPError as error:
        raise RuntimeError(f"{error.code}: {error.read().decode()[:300]}") from None
    took = (time.perf_counter() - started) * 1000
    if not answer.get("ok"):
        raise RuntimeError(answer)
    return answer["result"], took


def read(query):
    return call("QUERY", {"schema": "", "query": query})


def snap(point):
    q = (f"query {{ Junction(d: true) order by @distance(at, @point({point[0]}, {point[1]})) limit 1 "
         f"{{ osm }} }}")
    result, _ = read(q)
    return (result[0] if isinstance(result, list) else result)["osm"]


def pct(values, p):
    ordered = sorted(values)
    return ordered[min(len(ordered) - 1, round(p / 100 * (len(ordered) - 1)))]


def summary(latencies, wall):
    return {"count": len(latencies), "per_s": round(len(latencies) / wall, 2),
            "p50_ms": round(statistics.median(latencies), 1), "p95_ms": round(pct(latencies, 95), 1)}


def main():
    a, b = snap(FROM), snap(TO)
    route = f"query {{ Junction(osm: {a}) {{ drive *path by &cost toward at -> Junction(osm: {b}) {{ osm }} }} }}"

    def cost():
        result, took = read(route)
        path = result["drive"] if "drive" in result else result["Junction"]["drive"]
        return path["cost"], took

    expected, warm = cost()
    out = {"url": URL, "from_osm": a, "to_osm": b, "cost": expected, "warm_ms": round(warm, 1), "clients": []}
    print(json.dumps({"cost": expected, "warm_ms": round(warm, 1)}), flush=True)
    for clients in () if "--mixed-only" in sys.argv else (1, 4, 8):
        def one(_):
            got, took = cost()
            if got != expected:
                raise RuntimeError(f"cost {got} != {expected}")
            return took
        started = time.perf_counter()
        with cf.ThreadPoolExecutor(clients) as pool:
            latencies = list(pool.map(one, range(clients * 8)))
        row = {"clients": clients, **summary(latencies, time.perf_counter() - started)}
        out["clients"].append(row)
        print(json.dumps(row), flush=True)

    if MIXED:
        stop = threading.Event()
        reads, writes, failures = [], [], []
        lock = threading.Lock()

        def reader():
            while not stop.is_set():
                try:
                    got, took = cost()
                    if got != expected:
                        failures.append(f"cost {got} != {expected}")
                    with lock:
                        reads.append(took)
                except Exception as error:  # noqa: BLE001 - recorded, reported, fails the run
                    failures.append(repr(error))

        def writer():
            n = 0
            while not stop.is_set():
                osm = PROBE_BASE - n
                q = (f"mutation {{ Junction(osm: {osm} && at: @point(49.0, -110.0) && d: false && w: false && b: false)"
                     f" {{ osm }} }}")
                try:
                    _, took = call("POST", {"schema": "", "query": q})
                    writes.append(took)
                    seen, _ = read(f"query {{ Junction(osm: {osm}) {{ osm }} }}")
                    seen = seen[0] if isinstance(seen, list) and seen else seen
                    if not seen or seen.get("osm") != osm:
                        failures.append(f"probe {osm} not visible after its write was answered: {seen}")
                except Exception as error:  # noqa: BLE001
                    failures.append(repr(error))
                n += 1

        threads = [threading.Thread(target=reader) for _ in range(4)] + [threading.Thread(target=writer)]
        started = time.perf_counter()
        for thread in threads:
            thread.start()
        time.sleep(MIXED)
        stop.set()
        for thread in threads:
            thread.join()
        wall = time.perf_counter() - started
        # Probes by their unique osm (a scan of every junction would exceed
        # the traversal budget), a few past the last one tried.
        stored = 0
        for n in range(len(writes) + len(failures) + 5):
            seen, _ = read(f"query {{ Junction(osm: {PROBE_BASE - n}) {{ osm }} }}")
            stored += bool(seen[0] if isinstance(seen, list) and seen else seen)
        out["mixed"] = {"seconds": MIXED, "reads": summary(reads, wall) if reads else None,
                        "writes": summary(writes, wall) if writes else None,
                        "probes_stored": stored, "writes_answered": len(writes), "failures": failures[:20]}
        print(json.dumps(out["mixed"]), flush=True)
        if failures or stored != len(writes):
            with open(OUT, "w") as f:
                json.dump(out, f, indent=1)
            sys.exit("mixed run failed: see failures / probe count")

    with open(OUT, "w") as f:
        json.dump(out, f, indent=1)


if __name__ == "__main__":
    main()
