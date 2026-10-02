#!/bin/bash
# run.sh — the zega vs Neo4j benchmark, end to end.
#
#   ./run.sh [sizes...]        default: 10000 100000 1000000
#
# What it does, per size:
#   gen the dataset (deterministic)
#   zega-server: idle RAM empty → load → idle RAM loaded → throughput
#   Neo4j Docker (default memory): idle RAM empty → load → idle → throughput
#   Neo4j Docker (tuned memory): smallest pagecache that loads → idle RAM
#   Neo4j native, when a JRE is available (JAVA_HOME or Homebrew openjdk@21):
#     idle RAM empty → load → idle RAM loaded → throughput
#
# Raw numbers land in results/*.csv; this script only appends. Servers and
# containers it starts are stopped at the end; the only containers it removes
# are the kimi-neo4j-bench* ones. Everything listens on 127.0.0.1.
#
# Environment knobs:
#   NEO4J_IMAGE   default neo4j:5.26-community
#   NEO4J_PASS    default benchpass123
#   ZEGA_PORT     default 9399
#   SKIP_NATIVE=1 skip the native Neo4j runs
#   SKIP_TUNED=1  skip the tuned-memory Docker runs
#   CELL_SECONDS  default 30 (the measured window per cell)

set -u
cd "$(dirname "$0")"
ROOT=$(cd ../.. && pwd)
WORK=$PWD/.work
RESULTS=$PWD/results
NEO4J_IMAGE=${NEO4J_IMAGE:-neo4j:5.26-community}
NEO4J_PASS=${NEO4J_PASS:-benchpass123}
ZPORT=${ZEGA_PORT:-9399}
CELL=${CELL_SECONDS:-30}
BENCH=${BENCH:-$ROOT/.target/release/bench}
ZEGA=${ZEGA:-$ROOT/.target/release/zega}
mkdir -p "$WORK" "$RESULTS"

CONC="1 8 32"
READS="lookup onehop twohop filtered path"

mb() { awk -v k="$1" 'BEGIN { printf "%.1f", k / 1024 }'; }

# Median of the numbers on stdin (sample 3×, 5 s apart, at the call site).
median3() { sort -n | sed -n 2p; }

json_field() { # json text, key → value (loader output is one flat object)
  echo "$1" | sed -n "s/.*\"$2\":\([0-9]*\).*/\1/p"
}

docker_stats_mb() {
  # "1.234GiB / 7.753GiB" → MiB of the first field
  docker stats --no-stream --format '{{.MemUsage}}' "$1" 2>/dev/null | awk '{ split($1, u, /[a-zA-Z]+/); n = u[1]; unit = $1; sub(/[0-9.]+/, "", unit); if (unit == "GiB") n *= 1024; else if (unit == "KiB") n /= 1024; else if (unit == "B") n /= 1048576; printf "%.1f", n }'
}

container_ps_mb() {
  # RSS of the largest (java) process inside the container, MiB
  docker exec "$1" ps -eo rss=,comm= 2>/dev/null | awk '{ if ($1 > m) m = $1 } END { printf "%.1f", m / 1024 }'
}

container_disk_bytes() {
  # colima bind mounts don't show VM writes on the host: ask the container.
  docker exec "$1" du -sb /data 2>/dev/null | cut -f1
}

wait_http() { # url, timeout s
  local i=0
  while (( i < $2 )); do
    curl -sf --max-time 2 "$1" >/dev/null 2>&1 && return 0
    sleep 1; i=$((i + 1))
  done
  return 1
}

wait_bolt_docker() { # container, timeout s
  local i=0
  while (( i < $2 )); do
    docker exec "$1" cypher-shell -u neo4j -p "$NEO4J_PASS" "RETURN 1" >/dev/null 2>&1 && return 0
    sleep 2; i=$((i + 2))
  done
  return 1
}

wait_bolt_native() { # home, port, timeout s
  local i=0
  while (( i < $3 )); do
    NEO4J_HOME="$1" "$1/bin/cypher-shell" -a "bolt://127.0.0.1:$2" -u neo4j -p "$NEO4J_PASS" "RETURN 1" >/dev/null 2>&1 && return 0
    sleep 2; i=$((i + 2))
  done
  return 1
}

# macOS has no `timeout`: background + wait with a deadline.
with_timeout() { # seconds, command...
  local secs=$1; shift
  "$@" &
  local pid=$!
  local i=0
  while kill -0 "$pid" 2>/dev/null; do
    if (( i >= secs )); then kill "$pid" 2>/dev/null; wait "$pid" 2>/dev/null; return 124; fi
    sleep 5; i=$((i + 5))
  done
  wait "$pid"
}

zega_pid=""
neo4j_container=""
NATIVE_PID=""
cleanup() {
  [ -n "$zega_pid" ] && kill "$zega_pid" 2>/dev/null
  [ -n "$neo4j_container" ] && docker rm -f "$neo4j_container" >/dev/null 2>&1
  [ -n "$NATIVE_PID" ] && kill "$NATIVE_PID" 2>/dev/null
}
trap cleanup EXIT

log() { printf '[%s] %s\n' "$(date +%H:%M:%S)" "$*" | tee -a "$RESULTS/run.log"; }

throughput() { # engine, url-or-hostport, tag
  local engine=$1 target=$2 tag=$3 q c
  for q in $READS; do
    for c in $CONC; do
      log "  cell $tag $q conc=$c"
      "$BENCH" run --engine "$engine" --url "$target" --password "$NEO4J_PASS" \
        --dataset "$DS" --query "$q" --conc "$c" --seconds "$CELL" --warmup 3 \
        --tag "$tag" | grep '^RESULT' >> "$RESULTS/throughput.csv" || log "  CELL FAILED $tag $q conc=$c"
    done
  done
  local wb=$((100000000 + SIZE_ORD * 40000000))
  for c in $CONC; do
    log "  cell $tag write conc=$c"
    "$BENCH" run --engine "$engine" --url "$target" --password "$NEO4J_PASS" \
      --dataset "$DS" --query write --conc "$c" --seconds "$CELL" --warmup 3 \
      --write-base "$wb" --tag "$tag" | grep '^RESULT' >> "$RESULTS/throughput.csv" || log "  CELL FAILED $tag write conc=$c"
    wb=$((wb + 10000000))
  done
}

start_zega() { # data dir → waits healthy, sets zega_pid
  "$ZEGA" start --data "$1" --port "$ZPORT" --host 127.0.0.1 --query-time-limit 0 > "$WORK/zega.log" 2>&1 &
  zega_pid=$!
  wait_http "http://127.0.0.1:$ZPORT/health" 60 || { log "zega did not start; see $WORK/zega.log"; exit 1; }
}

stop_zega() {
  [ -n "$zega_pid" ] && kill "$zega_pid" 2>/dev/null
  wait "$zega_pid" 2>/dev/null
  zega_pid=""
}

: > "$RESULTS/run.log"
echo "tag,engine,mode,size,state,rss_mb,ondisk_mb,notes" > "$RESULTS/idle.csv"
echo "tag,engine,query,concurrency,window_s,ops,errors,qps,p50_ms,p95_ms,p99_ms" > "$RESULTS/throughput.csv"
echo "engine,mode,size,load_ms,on_disk_bytes,index_await_ms,extra" > "$RESULTS/load.csv"

JAVA_BIN=""
if [ "${SKIP_NATIVE:-0}" != "1" ]; then
  if [ -n "${JAVA_HOME:-}" ] && [ -x "${JAVA_HOME}/bin/java" ]; then
    JAVA_BIN="$JAVA_HOME/bin/java"
  elif [ -x /usr/local/opt/openjdk@21/bin/java ]; then
    JAVA_BIN=/usr/local/opt/openjdk@21/bin/java
  elif [ -x /opt/homebrew/opt/openjdk@21/bin/java ]; then
    JAVA_BIN=/opt/homebrew/opt/openjdk@21/bin/java
  fi
  [ -z "$JAVA_BIN" ] && log "no JRE found: native Neo4j runs skipped"
fi
[ -x "$BENCH" ] || { log "missing $BENCH (cargo build --release -p zega-cli; build this crate first)"; exit 1; }
[ -x "$ZEGA" ] || { log "missing $ZEGA"; exit 1; }

NATIVE_HOME=""
if [ -n "$JAVA_BIN" ]; then
  NATIVE_HOME=$WORK/neo4j-native-dist
  if [ ! -d "$NATIVE_HOME/bin" ]; then
    log "downloading Neo4j community tarball for the native runs"
    mkdir -p "$NATIVE_HOME"
    curl -fsSL https://dist.neo4j.org/neo4j-community-5.26.0-unix.tar.gz | tar -xz --strip-components=1 -C "$NATIVE_HOME" \
      || { log "native distro download failed; native runs skipped"; NATIVE_HOME=""; }
  fi
fi

SIZES="$*"
[ -z "$SIZES" ] && SIZES="10000 100000 1000000"
SIZE_ORD=0
for SIZE in $SIZES; do
  SIZE_ORD=$((SIZE_ORD + 1))
  log "=== size $SIZE ==="
  DS=$WORK/ds-$SIZE
  rm -rf "$DS" && "$BENCH" gen "$SIZE" "$DS" >/dev/null || { log "gen failed"; exit 1; }

  # ---------------- zega-server ----------------
  ZDATA=$WORK/zega-$SIZE
  rm -rf "$ZDATA"
  log "zega: idle, empty database"
  start_zega "$ZDATA"
  sleep 60
  rss=$( { ps -o rss= -p "$zega_pid" | tr -d ' '; sleep 5; ps -o rss= -p "$zega_pid" | tr -d ' '; sleep 5; ps -o rss= -p "$zega_pid" | tr -d ' '; } | median3 )
  disk=$(du -sk "$ZDATA" | cut -f1)
  echo "z$SIZE,zega,default,$SIZE,empty,$(mb "$rss"),$(mb "$disk")," >> "$RESULTS/idle.csv"
  log "zega empty: rss $(mb "$rss") MB"
  stop_zega

  log "zega: load"
  load=$("$BENCH" load-zega --dataset "$DS" --data "$ZDATA")
  apply=$(json_field "$load" apply_ms)
  ckpt=$(json_field "$load" checkpoint_ms)
  disk=$(json_field "$load" on_disk_bytes)
  echo "zega,default,$SIZE,$apply,$disk,0,checkpoint_ms=$ckpt" >> "$RESULTS/load.csv"
  log "zega loaded: $load"

  log "zega: idle, loaded"
  start_zega "$ZDATA"
  sleep 60
  rss=$( { ps -o rss= -p "$zega_pid" | tr -d ' '; sleep 5; ps -o rss= -p "$zega_pid" | tr -d ' '; sleep 5; ps -o rss= -p "$zega_pid" | tr -d ' '; } | median3 )
  disk=$(du -sk "$ZDATA" | cut -f1)
  echo "z$SIZE,zega,default,$SIZE,loaded,$(mb "$rss"),$(mb "$disk")," >> "$RESULTS/idle.csv"
  log "zega loaded: rss $(mb "$rss") MB, disk $(mb "$disk") MB"

  # ---------------- neo4j docker, default memory ----------------
  CDATA=$WORK/docker-data-$SIZE
  rm -rf "$CDATA" && mkdir -p "$CDATA"
  neo4j_container=kimi-neo4j-bench
  docker rm -f "$neo4j_container" >/dev/null 2>&1
  log "neo4j docker: idle, empty database"
  docker run -d --name "$neo4j_container" -p 127.0.0.1:7687:7687 -p 127.0.0.1:7474:7474 \
    -e NEO4J_AUTH=neo4j/$NEO4J_PASS -v "$CDATA:/data" "$NEO4J_IMAGE" >/dev/null
  wait_bolt_docker "$neo4j_container" 180 || { log "neo4j did not start"; exit 1; }
  sleep 60
  stats=$(docker_stats_mb "$neo4j_container")
  psmb=$(container_ps_mb "$neo4j_container")
  diskb=$(container_disk_bytes "$neo4j_container")
  echo "n$SIZE,neo4j,docker-default,$SIZE,empty,$psmb,$(mb $((${diskb:-0} / 1024))),stats=${stats}MB" >> "$RESULTS/idle.csv"
  log "neo4j docker empty: ps $psmb MB, stats $stats MB"

  log "neo4j docker: load"
  load=$("$BENCH" load-neo4j --dataset "$DS" --url 127.0.0.1:7687 --password "$NEO4J_PASS" --clear)
  lm=$(json_field "$load" load_ms)
  iam=$(json_field "$load" index_await_ms)
  nodes=$(json_field "$load" nodes)
  rels=$(json_field "$load" relationships)
  diskb=$(container_disk_bytes "$neo4j_container")
  echo "neo4j,docker-default,$SIZE,$lm,${diskb:-0},$iam,nodes=$nodes;rels=$rels" >> "$RESULTS/load.csv"
  log "neo4j docker loaded: $load"
  docker logs "$neo4j_container" 2>&1 | grep -iE "memory|heap|pagecache" | head -8 >> "$RESULTS/neo4j-memory-config.txt"

  sleep 60
  stats=$(docker_stats_mb "$neo4j_container")
  psmb=$(container_ps_mb "$neo4j_container")
  diskb=$(container_disk_bytes "$neo4j_container")
  echo "n$SIZE,neo4j,docker-default,$SIZE,loaded,$psmb,$(mb $((${diskb:-0} / 1024))),stats=${stats}MB" >> "$RESULTS/idle.csv"
  log "neo4j docker loaded: ps $psmb MB, stats $stats MB, disk $(mb $((${diskb:-0} / 1024))) MB"

  # ---------------- parity, once per size ----------------
  log "parity zega vs neo4j (docker)"
  if ! parity_out=$("$BENCH" parity --zega-url "http://127.0.0.1:$ZPORT" --neo4j-url 127.0.0.1:7687 \
      --password "$NEO4J_PASS" --dataset "$DS" --samples 25 2>&1); then
    echo "$parity_out" | tee -a "$RESULTS/run.log"
    log "PARITY FAILED at size $SIZE — timings would be meaningless"
    exit 1
  fi
  echo "$parity_out" | tee -a "$RESULTS/run.log" | grep -q ' 25/25' || true
  n_ok=$(echo "$parity_out" | grep -c 'agree')
  [ "$n_ok" = 6 ] || { log "PARITY INCOMPLETE at size $SIZE ($n_ok/6 kinds)"; exit 1; }

  # ---------------- throughput: zega then neo4j ----------------
  throughput zega "http://127.0.0.1:$ZPORT" "z$SIZE"
  throughput neo4j "127.0.0.1:7687" "n$SIZE"

  stop_zega
  rm -rf "$ZDATA"

  # ---------------- neo4j docker, tuned memory ----------------
  if [ "${SKIP_TUNED:-0}" != "1" ]; then
    # Smallest pagecache (of the candidates) that still loads the graph.
    TDATA=$WORK/docker-tuned-data-$SIZE
    loaded_pc=""
    for PC in 256m 512m 1g 2g 4g; do
      rm -rf "$TDATA" && mkdir -p "$TDATA"
      docker rm -f "$neo4j_container" >/dev/null 2>&1
      log "neo4j docker tuned: trying pagecache $PC"
      docker run -d --name "$neo4j_container" -p 127.0.0.1:7687:7687 -p 127.0.0.1:7474:7474 \
        -e NEO4J_AUTH=neo4j/$NEO4J_PASS \
        -e NEO4J_server_memory_heap_initial__size=512m \
        -e NEO4J_server_memory_heap_max__size=512m \
        -e NEO4J_server_memory_pagecache_size=$PC \
        -v "$TDATA:/data" "$NEO4J_IMAGE" >/dev/null
      if wait_bolt_docker "$neo4j_container" 180; then
        if with_timeout 1500 "$BENCH" load-neo4j --dataset "$DS" --url 127.0.0.1:7687 --password "$NEO4J_PASS" --clear > "$WORK/tuned-load.json" 2>&1; then
          loaded_pc=$PC
          lm=$(json_field "$(cat "$WORK/tuned-load.json")" load_ms)
          iam=$(json_field "$(cat "$WORK/tuned-load.json")" index_await_ms)
          diskb=$(container_disk_bytes "$neo4j_container")
          echo "neo4j,docker-tuned,$SIZE,$lm,${diskb:-0},$iam,pagecache=$PC" >> "$RESULTS/load.csv"
          break
        fi
      fi
      log "  pagecache $PC did not load in time"
    done
    if [ -n "$loaded_pc" ]; then
      sleep 60
      stats=$(docker_stats_mb "$neo4j_container")
      psmb=$(container_ps_mb "$neo4j_container")
      diskb=$(container_disk_bytes "$neo4j_container")
      echo "t$SIZE,neo4j,docker-tuned,$SIZE,loaded,$psmb,$(mb $((${diskb:-0} / 1024))),stats=${stats}MB;heap=512m;pagecache=$loaded_pc" >> "$RESULTS/idle.csv"
      log "neo4j docker tuned ($loaded_pc): ps $psmb MB, stats $stats MB"
    else
      log "  tuned: no pagecache candidate loaded"
    fi
    rm -rf "$TDATA"
  fi

  docker rm -f "$neo4j_container" >/dev/null 2>&1
  neo4j_container=""
  rm -rf "$CDATA"

  # ---------------- neo4j native, when a JRE is available ----------------
  if [ -n "$NATIVE_HOME" ]; then
    NH=$WORK/neo4j-native-$SIZE
    rm -rf "$NH"
    mkdir -p "$NH"
    cp -R "$NATIVE_HOME/." "$NH/"
    export JAVA_HOME="$(cd "$(dirname "$JAVA_BIN")/.." && pwd)"
    export PATH="$JAVA_HOME/bin:$PATH"
    cat >> "$NH/conf/neo4j.conf" <<'EOF'
server.bolt.listen_address=127.0.0.1:17687
server.http.listen_address=127.0.0.1:17474
EOF
    log "neo4j native: set initial password"
    NEO4J_HOME="$NH" "$NH/bin/neo4j-admin" dbms set-initial-password "$NEO4J_PASS" >> "$WORK/neo4j-native-$SIZE.log" 2>&1 \
      || log "set-initial-password failed (fresh home?)"
    log "neo4j native: idle, empty database"
    NEO4J_HOME="$NH" "$NH/bin/neo4j" start > "$WORK/neo4j-native-$SIZE.log" 2>&1
    wait_bolt_native "$NH" 17687 180 || { log "native neo4j did not start; see $WORK/neo4j-native-$SIZE.log"; exit 1; }
    NATIVE_PID=$(ps -eo pid,args | grep "[o]rg.neo4j.server" | awk '{print $1; exit}')
    sleep 60
    rss=$( { ps -o rss= -p "$NATIVE_PID" | tr -d ' '; sleep 5; ps -o rss= -p "$NATIVE_PID" | tr -d ' '; sleep 5; ps -o rss= -p "$NATIVE_PID" | tr -d ' '; } | median3 )
    disk=$(du -sk "$NH/data" 2>/dev/null | cut -f1)
    echo "N$SIZE,neo4j,native,$SIZE,empty,$(mb "$rss"),$(mb "$disk")," >> "$RESULTS/idle.csv"
    log "neo4j native empty: rss $(mb "$rss") MB"

    log "neo4j native: load"
    load=$("$BENCH" load-neo4j --dataset "$DS" --url 127.0.0.1:17687 --password "$NEO4J_PASS" --clear)
    lm=$(json_field "$load" load_ms)
    iam=$(json_field "$load" index_await_ms)
    nodes=$(json_field "$load" nodes)
    rels=$(json_field "$load" relationships)
    disk=$(du -sk "$NH/data" | cut -f1)
    echo "neo4j,native,$SIZE,$lm,$((disk * 1024)),$iam,nodes=$nodes;rels=$rels" >> "$RESULTS/load.csv"
    log "neo4j native loaded: $load"

    sleep 60
    rss=$( { ps -o rss= -p "$NATIVE_PID" | tr -d ' '; sleep 5; ps -o rss= -p "$NATIVE_PID" | tr -d ' '; sleep 5; ps -o rss= -p "$NATIVE_PID" | tr -d ' '; } | median3 )
    disk=$(du -sk "$NH/data" 2>/dev/null | cut -f1)
    echo "N$SIZE,neo4j,native,$SIZE,loaded,$(mb "$rss"),$(mb "$disk")," >> "$RESULTS/idle.csv"
    log "neo4j native loaded: rss $(mb "$rss") MB, disk $(mb "$disk") MB"
    grep -iE "memory|heap|pagecache" "$NH/logs/neo4j.log" 2>/dev/null | head -8 >> "$RESULTS/neo4j-memory-config.txt"

    throughput neo4j "127.0.0.1:17687" "N$SIZE"

    NEO4J_HOME="$NH" "$NH/bin/neo4j" stop >> "$WORK/neo4j-native-$SIZE.log" 2>&1
    NATIVE_PID=""
    rm -rf "$NH"
  fi

  rm -rf "$DS"
  log "=== size $SIZE done ==="
done

log "ALL DONE"
