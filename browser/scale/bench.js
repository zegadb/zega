// The scale bench's browser path (zegadb/zega#143): the wasm engine loading
// a travel sample exactly as zega.earth loads one — the explorer's formula
// (zegadb-earth src-client/engine.js loadSample): init the wasm package,
// read the .zql, fetch every CSV the engine's load_locations names, apply
// with apply_with_sources. Then the sample's fixed query set, 20 runs each.
//
// The driver serves this file and calls window.runScaleBench(nodes) with the
// node count (1000, 10000, ...); data lives at /data/<nodes>/.

import init, { ZegaWasm } from '../pkg/zega_wasm.js';

const wasmReady = init();
const QUERY_RUNS = 20;

function percentile(sorted, p) {
  return sorted[Math.round((sorted.length - 1) * p)];
}

function summarize(samples) {
  const sorted = [...samples].sort((a, b) => a - b);
  return { runs: sorted.length, p50_ms: percentile(sorted, 0.5), p95_ms: percentile(sorted, 0.95), p99_ms: percentile(sorted, 0.99) };
}

function jsHeap() {
  return performance.memory ? performance.memory.usedJSHeapSize : null;
}

window.runScaleBench = async (nodes) => {
  const wasm = await wasmReady;
  const base = `/data/${nodes}`;
  const started = performance.now();

  // loadSample, timed in its two phases: download (zql + CSV shards) and
  // apply (parse + insert, all inside the engine).
  const source = await (await fetch(`${base}/travel.zql`)).text();
  const probe = new ZegaWasm();
  const locations = JSON.parse(probe.load_locations(source, true));
  probe.free();
  const sources = {};
  let downloadBytes = 0;
  const fetchStarted = performance.now();
  await Promise.all(locations.map(async (location) => {
    const response = await fetch(`${base}/${location.replace(/^\.\//, '')}`);
    if (!response.ok) throw new Error(`Cannot read ${location}: HTTP ${response.status}`);
    const text = await response.text();
    downloadBytes += new TextEncoder().encode(text).length;
    sources[location] = text;
  }));
  const fetched = performance.now();

  const db = new ZegaWasm();
  const heapBeforeApply = jsHeap();
  db.apply_with_sources(source, JSON.stringify(sources));
  const applied = performance.now();
  // The shard texts are transport only; drop them before measuring rest.
  for (const key of Object.keys(sources)) delete sources[key];

  const heapAfterLoad = jsHeap();
  const wasmBytes = wasm.memory.buffer.byteLength;

  // The fixed query set, the same statements the server path runs.
  const queries = await (await fetch(`${base}/queries.json`)).json();
  const timings = {};
  let firstUsable = null;
  for (const name of ['search', 'neighbours', 'near']) {
    const texts = queries[name];
    const warm = JSON.parse(db.run(source, texts[0])); // warm run, like zega-mem's
    if (warm === null) throw new Error(`${name}: warm run matched nothing`);
    if (firstUsable === null) firstUsable = performance.now();
    const samples = [];
    for (const text of texts) {
      const t = performance.now();
      db.run(source, text);
      samples.push(performance.now() - t);
    }
    timings[name] = summarize(samples);
  }
  const heapAfterQueries = jsHeap();

  return {
    path: 'browser',
    nodes,
    download_bytes: downloadBytes + new TextEncoder().encode(source).length,
    shards: locations.length,
    download_ms: fetched - started,
    load_ms: applied - fetched,
    first_usable_ms: firstUsable - started,
    js_heap_after_load_bytes: heapAfterLoad !== null && heapBeforeApply !== null ? heapAfterLoad : null,
    js_heap_after_queries_bytes: heapAfterQueries,
    wasm_memory_bytes: wasmBytes,
    queries: timings,
  };
};
