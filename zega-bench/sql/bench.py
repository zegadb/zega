#!/usr/bin/env python3
"""APS 54 benchmark: Zega + SQLite against Zega + Postgres (pg_zega), on one
machine, one after the other, each with a fresh data directory.

  python3 zega-bench/sql/bench.py --zega-server .target/release/zega-server \
      --image addiscorp/pg_zega:17 --writes 5000 --reads 5000 --out bench.json

Three setups, the same table `users(user_id, name, email)` and the same rows:
  zega+sqlite         zega-server --storage sqlite. A write is one
                      POST /transaction: the SQL INSERT and the graph node,
                      one commit.
  zega+postgres       pg_zega's precommit mode: an INSERT into a tracked
                      table; Postgres's commit waits for Zega.
  zega+postgres-jrnl  pg_zega's journal mode: the commit writes zega.journal;
                      the delivery worker sends it to Zega afterwards.

Measured: write latency and throughput (one row per transaction, one client),
SQL reads, ZQL reads and mixed reads by key, how long the graph trails the SQL
commit, how fast a backlog drains, and resident memory idle and after the
writes. Every client talks to a server over loopback: HTTP keep-alive for
Zega, one psycopg connection for Postgres. Durability is each server's
default: Zega waits for its commit to reach the disk, Postgres runs with
fsync on.
"""
import argparse
import http.client
import json
import os
import platform
import subprocess
import sys
import tempfile
import time

TOKEN = "zega-bench-token"
SCHEMA = "schema { type users { user_id: Int name: String email: String } } unique { users { user_id } }"


def ms(ns):
    return round(ns / 1e6, 3)


def summary(samples_ns, seconds=None):
    s = sorted(samples_ns)
    pick = lambda p: s[min(len(s) - 1, round((len(s) - 1) * p))]
    out = {"n": len(s), "p50_ms": ms(pick(0.5)), "p99_ms": ms(pick(0.99)), "max_ms": ms(s[-1])}
    if seconds is not None:
        out["per_sec"] = round(len(s) / seconds)
    return out


def timed(n, op):
    samples = []
    start = time.perf_counter()
    for i in range(n):
        t = time.perf_counter_ns()
        op(i)
        samples.append(time.perf_counter_ns() - t)
    return summary(samples, time.perf_counter() - start)


class Zega:
    def __init__(self, binary, port, storage, workdir):
        self.port = port
        data = os.path.join(workdir, f"zega-{storage}")
        token = os.path.join(workdir, "token")
        with open(token, "w") as f:
            f.write(TOKEN + "\n")
        self.log = open(os.path.join(workdir, f"zega-{storage}.log"), "w")
        self.proc = subprocess.Popen(
            [binary, "start", "--data", data, "--host", "0.0.0.0", "--port", str(port),
             "--token-file", token, "--storage", storage],
            stdout=self.log, stderr=subprocess.STDOUT)
        self.conn = None
        for _ in range(100):
            try:
                self.call("GET", "/health")
                return
            except OSError:
                self.conn = None
                time.sleep(0.1)
        sys.exit(f"zega-server ({storage}) did not start; see {self.log.name}")

    def call(self, method, path, body=None):
        if self.conn is None:
            self.conn = http.client.HTTPConnection("127.0.0.1", self.port, timeout=30)
        headers = {"Authorization": f"Bearer {TOKEN}", "Accept": "application/json"}
        data = None
        if body is not None:
            data = json.dumps(body)
            headers["Content-Type"] = "application/json"
        self.conn.request(method, path, data, headers)
        response = self.conn.getresponse()
        payload = response.read()
        if response.status != 200:
            raise RuntimeError(f"{method} {path}: {response.status} {payload[:300]!r}")
        return json.loads(payload) if payload else None

    def zql(self, query):
        return self.call("POST", "/zql", {"query": query})["result"]

    def rss_mib(self):
        out = subprocess.run(["ps", "-o", "rss=", "-p", str(self.proc.pid)], capture_output=True, text=True).stdout
        return round(int(out.strip()) / 1024, 1)

    def stop(self):
        self.proc.terminate()
        self.proc.wait(timeout=30)
        self.log.close()


def user_node(i):
    return f'query {{ users(user_id = {i}) {{ name }} }}'


def zega_sqlite(args, workdir):
    zega = Zega(args.zega_server, args.port, "sqlite", workdir)
    time.sleep(2)
    idle = zega.rss_mib()
    zega.call("POST", "/sql", {"sql": "CREATE TABLE users (user_id INTEGER PRIMARY KEY, name TEXT NOT NULL, email TEXT NOT NULL)"})
    zega.call("PUT", "/schema", {"schema": SCHEMA})

    def write(i):
        zega.call("POST", "/transaction", {"steps": [
            {"sql": "INSERT INTO users (user_id, name, email) VALUES (?1, ?2, ?3)", "params": [i, f"user {i}", f"u{i}@example.com"]},
            {"zql": f'mutation {{ users(user_id: {i} && name: "user {i}" && email: "u{i}@example.com") {{ user_id }} }}'},
        ]})

    result = {"writes": timed(args.writes, write)}
    sql_read = lambda i: zega.call("POST", "/sql", {"sql": "SELECT name FROM users WHERE user_id = ?1", "params": [i % args.writes]})
    zql_read = lambda i: zega.zql(user_node(i % args.writes))
    result["sql_reads"] = timed(args.reads, sql_read)
    result["zql_reads"] = timed(args.reads, zql_read)
    result["mixed_reads"] = timed(args.reads, lambda i: (sql_read if i % 2 else zql_read)(i))
    assert zega.zql(user_node(args.writes - 1)) == {"name": f"user {args.writes - 1}"}
    result["graph_lag"] = "0 (the row and the node are one commit)"
    result["ram_mib"] = {"idle": idle, "after_writes": zega.rss_mib()}
    zega.stop()
    return result


class Postgres:
    def __init__(self, image, port, zega_port, mode):
        import psycopg
        self.name = f"zega-bench-pg-{os.getpid()}"
        subprocess.run(["docker", "rm", "-f", self.name], capture_output=True)
        subprocess.run(
            ["docker", "run", "-d", "--name", self.name, "--add-host", "host.docker.internal:host-gateway",
             "-e", "POSTGRES_PASSWORD=bench", "-e", f"ZEGA_PASSWORD={TOKEN}",
             "-e", f"ZEGA_URL=http://host.docker.internal:{zega_port}", "-p", f"127.0.0.1:{port}:5432", image],
            check=True, capture_output=True)
        for _ in range(120):
            try:
                self.conn = psycopg.connect(f"host=127.0.0.1 port={port} user=postgres password=bench", autocommit=True)
                self.conn.execute("SELECT 1")
                break
            except psycopg.OperationalError:
                time.sleep(0.5)
        else:
            sys.exit("postgres did not start")
        # The image's first start runs a setup server, then the real one.
        time.sleep(2)
        self.conn = psycopg.connect(f"host=127.0.0.1 port={port} user=postgres password=bench", autocommit=True)
        if mode == "journal":
            for _ in range(120):
                if self.one("SELECT count(*) FROM pg_replication_slots WHERE slot_name = 'zega'") == 1:
                    break
                time.sleep(0.5)
            else:
                sys.exit("the delivery worker created no slot")
            self.conn.execute("ALTER DATABASE postgres SET zega.mode = 'journal'")
            self.conn = psycopg.connect(f"host=127.0.0.1 port={port} user=postgres password=bench", autocommit=True)
            assert self.one("SHOW zega.mode") == "journal"

    def one(self, sql, params=None):
        return self.conn.execute(sql, params).fetchone()[0]

    def mem_mib(self):
        out = subprocess.run(["docker", "stats", "--no-stream", "--format", "{{.MemUsage}}", self.name],
                             capture_output=True, text=True, check=True).stdout.split("/")[0].strip()
        number, unit = float(out[:-3]), out[-3:]
        return round(number * {"KiB": 1 / 1024, "MiB": 1, "GiB": 1024}[unit], 1)

    def stop(self):
        self.conn.close()
        subprocess.run(["docker", "rm", "-f", self.name], capture_output=True, check=True)


def zega_postgres(args, workdir, mode):
    zega = Zega(args.zega_server, args.port, "file", workdir + f"/{mode}")
    pg = Postgres(args.image, args.pg_port, args.port, mode)
    time.sleep(2)
    idle = {"postgres": pg.mem_mib(), "zega": zega.rss_mib()}
    pg.conn.execute("CREATE EXTENSION IF NOT EXISTS pg_zega")
    pg.conn.execute("SET zega.password = %s" % psql_literal(TOKEN))
    pg.conn.execute("CREATE TABLE users (user_id int PRIMARY KEY, name text NOT NULL, email text NOT NULL)")
    pg.conn.execute("SELECT zega.track('users')")
    insert = "INSERT INTO users (user_id, name, email) VALUES (%s, %s, %s)"

    result = {"writes": timed(args.writes, lambda i: pg.conn.execute(insert, (i, f"user {i}", f"u{i}@example.com")))}
    if mode == "journal":
        start = time.perf_counter()
        waiting = pg.one("SELECT count(*) FROM zega.journal")
        while pg.one("SELECT count(*) FROM zega.journal") > 0:
            time.sleep(0.01)
        drained = time.perf_counter() - start
        result["backlog_after_writes"] = {"rows": waiting, "drained_s": round(drained, 2),
                                         "rows_per_sec": round(waiting / drained) if drained > 0 else None}
    sql_read = lambda i: pg.one("SELECT name FROM users WHERE user_id = %s", (i % args.writes,))
    zql_read = lambda i: zega.zql(user_node(i % args.writes))
    result["sql_reads"] = timed(args.reads, sql_read)
    result["zql_reads"] = timed(args.reads, zql_read)
    result["mixed_reads"] = timed(args.reads, lambda i: (sql_read if i % 2 else zql_read)(i))
    assert zega.zql(user_node(args.writes - 1)) == {"name": f"user {args.writes - 1}"}
    if mode == "journal":
        lags = []
        for k in range(args.lag_samples):
            i = args.writes + k
            pg.conn.execute(insert, (i, f"user {i}", f"u{i}@example.com"))
            t = time.perf_counter_ns()
            while not zega.zql(user_node(i)):
                time.sleep(0.001)
            lags.append(time.perf_counter_ns() - t)
        result["graph_lag"] = summary(lags)
    else:
        result["graph_lag"] = "0 (Postgres's commit waits for Zega)"
    result["ram_mib"] = {"idle": idle, "after_writes": {"postgres": pg.mem_mib(), "zega": zega.rss_mib()}}
    pg.stop()
    zega.stop()
    return result


def psql_literal(value):
    return "'" + value.replace("'", "''") + "'"


def machine():
    info = {"python": platform.python_version(), "os": platform.platform()}
    for cmd, key in ((["nproc"], "cpus"), (["sh", "-c", "lscpu | grep 'Model name' | sed 's/.*: *//'"], "cpu"),
                     (["sh", "-c", "free -m | awk '/Mem:/ {print $2}'"], "ram_mib"),
                     (["docker", "version", "--format", "{{.Server.Version}}"], "docker")):
        try:
            info[key] = subprocess.run(cmd, capture_output=True, text=True, timeout=10).stdout.strip()
        except OSError:
            pass
    return info


def table(results):
    rows = [("", *results)]
    def cell(setup, key, field):
        value = results[setup].get(key)
        return value if isinstance(value, str) else f"{value[field]}" if value else "—"
    for key, label in (("writes", "write"), ("sql_reads", "SQL read"), ("zql_reads", "ZQL read"), ("mixed_reads", "mixed read")):
        rows.append((f"{label} p50 / p99 ms", *(f"{cell(s, key, 'p50_ms')} / {cell(s, key, 'p99_ms')}" for s in results)))
        rows.append((f"{label} per second", *(cell(s, key, "per_sec") for s in results)))
    rows.append(("graph lag p50 / p99 ms", *(results[s]["graph_lag"] if isinstance(results[s]["graph_lag"], str)
                                            else f"{results[s]['graph_lag']['p50_ms']} / {results[s]['graph_lag']['p99_ms']}" for s in results)))
    def ram(setup, when):
        value = results[setup]["ram_mib"][when]
        return f"{value}" if not isinstance(value, dict) else f"{round(value['postgres'] + value['zega'], 1)} ({value['postgres']} + {value['zega']})"
    rows.append(("RAM idle MiB", *(ram(s, "idle") for s in results)))
    rows.append(("RAM after writes MiB", *(ram(s, "after_writes") for s in results)))
    lines = ["| " + " | ".join(map(str, row)) + " |" for row in rows]
    lines.insert(1, "|" + "---|" * len(rows[0]))
    return "\n".join(lines)


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--zega-server", required=True)
    parser.add_argument("--image", default="addiscorp/pg_zega:17")
    parser.add_argument("--writes", type=int, default=5000)
    parser.add_argument("--reads", type=int, default=5000)
    parser.add_argument("--lag-samples", type=int, default=200)
    parser.add_argument("--port", type=int, default=9371)
    parser.add_argument("--pg-port", type=int, default=5471)
    parser.add_argument("--out", required=True)
    args = parser.parse_args()
    args.zega_server = os.path.abspath(args.zega_server)
    version = subprocess.run([args.zega_server, "--version"], capture_output=True, text=True).stdout.strip()
    with tempfile.TemporaryDirectory(dir=os.environ.get("TMPDIR")) as workdir:
        os.makedirs(workdir + "/precommit")
        os.makedirs(workdir + "/journal")
        results = {
            "zega+sqlite": zega_sqlite(args, workdir),
            "zega+postgres": zega_postgres(args, workdir, "precommit"),
            "zega+postgres-jrnl": zega_postgres(args, workdir, "journal"),
        }
    report = {"zega_server": version, "image": args.image, "writes": args.writes, "reads": args.reads,
              "lag_samples": args.lag_samples, "machine": machine(), "results": results}
    with open(args.out, "w") as f:
        json.dump(report, f, indent=2)
    print(table(results))
    print(f"\n{json.dumps(report['machine'])}\nzega-server {version}, {args.image}")


if __name__ == "__main__":
    main()
