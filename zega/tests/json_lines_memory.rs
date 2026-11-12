//! A bulk load needs about the memory of the graph it builds (zegadb/zega#128:
//! loading through ZQL took ~2x the graph's resting memory). A JSON-lines
//! load into a disk database is measured against the same database reopened
//! from disk, the graph's resting size, on both storages.
//!
//! Heap is measured with a counting global allocator: exact, the same on
//! every OS, and the part of RSS a load can grow. This file is its own test
//! binary with one test, so nothing else allocates while it measures. The
//! input is generated line by line on another thread through a pipe, so it
//! is never in memory as a whole: neither is it in the loader.

use std::alloc::{GlobalAlloc, Layout, System};
use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};

use zega::graph_file::json_lines::JsonLinesOptions;
use zega::{Storage, Zega};

struct Counting;

static CURRENT: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            let now = CURRENT.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
            PEAK.fetch_max(now, Ordering::Relaxed);
        }
        ptr
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        CURRENT.fetch_sub(layout.size(), Ordering::Relaxed);
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let new = unsafe { System.realloc(ptr, layout, new_size) };
        if !new.is_null() {
            if new_size >= layout.size() {
                let now = CURRENT.fetch_add(new_size - layout.size(), Ordering::Relaxed)
                    + (new_size - layout.size());
                PEAK.fetch_max(now, Ordering::Relaxed);
            } else {
                CURRENT.fetch_sub(layout.size() - new_size, Ordering::Relaxed);
            }
        }
        new
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn current() -> usize {
    CURRENT.load(Ordering::Relaxed)
}

/// Start a new peak measurement from what is allocated now.
fn reset_peak() -> usize {
    let now = current();
    PEAK.store(now, Ordering::Relaxed);
    now
}

fn peak() -> usize {
    PEAK.load(Ordering::Relaxed)
}

const NODES: u64 = 200_000;
const RELATIONSHIPS: u64 = 300_000;
const MIB: f64 = 1024.0 * 1024.0;

/// #128's acceptance: memory during a load within ~20-25% of the graph's
/// resting memory. A load also holds fixed buffers (a line, 64 KiB reads,
/// the `.graph` writer's), which don't grow with the graph: this much.
const FIXED: usize = 8 << 20;

/// `NODES` items with a key, a name and a number, then `RELATIONSHIPS`
/// links between pseudo-random items: the shape an intake writes.
fn write_lines(out: impl Write) -> std::io::Result<()> {
    let mut out = std::io::BufWriter::new(out);
    for id in 1..=NODES {
        writeln!(
            out,
            r#"{{"type":"node","id":{id},"labels":["Item"],"props":{{"qid":"Q{id}","name":"item {id:x}","pop":{}}}}}"#,
            id * 7
        )?;
    }
    let mut state = 0x0127_0128u64;
    for _ in 0..RELATIONSHIPS {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let (from, to) = ((state >> 33) % NODES + 1, (state >> 13) % NODES + 1);
        writeln!(out, r#"{{"type":"rel","kind":"LINK","from":{from},"to":{to}}}"#)?;
    }
    out.flush()
}

/// The heap a load into a new database in `dir` peaked at, and what the
/// loaded database holds afterwards.
fn load(dir: &std::path::Path, storage: Storage) -> (usize, usize) {
    let base = reset_peak();
    let zega = Zega::open(dir.to_str().unwrap()).storage(storage).snapshot_every(0).build().unwrap();
    let (reader, writer) = std::io::pipe().unwrap();
    let producer = std::thread::spawn(move || write_lines(writer));
    let summary = zega.import_json_lines(reader, &JsonLinesOptions::default()).unwrap();
    producer.join().unwrap().unwrap();
    assert_eq!((summary.nodes, summary.relationships), (NODES, RELATIONSHIPS));
    let held = current() - base;
    let load_peak = peak() - base;
    drop(zega);
    (load_peak, held)
}

/// The heap the same database holds once reopened from disk.
fn resting(dir: &std::path::Path, storage: Storage) -> usize {
    let base = reset_peak();
    let zega = Zega::open(dir.to_str().unwrap()).storage(storage).snapshot_every(0).build().unwrap();
    assert_eq!(zega.counts().unwrap().nodes as u64, NODES);
    let resting = current() - base;
    drop(zega);
    resting
}

#[test]
fn a_bulk_load_peaks_close_to_the_graphs_resting_memory() {
    let mut storages = vec![Storage::File];
    #[cfg(feature = "sqlite")]
    storages.push(Storage::Sqlite);
    for storage in storages {
        let dir = tempfile::tempdir().unwrap();
        let (load_peak, held) = load(dir.path(), storage);
        let resting = resting(dir.path(), storage);
        println!(
            "{storage:?}: {NODES} nodes, {RELATIONSHIPS} relationships: resting {:.1} MiB, held after load {:.1} MiB, load peak {:.1} MiB ({:.3}x resting)",
            resting as f64 / MIB,
            held as f64 / MIB,
            load_peak as f64 / MIB,
            load_peak as f64 / resting as f64
        );
        let bound = resting + resting / 4 + FIXED;
        assert!(
            load_peak <= bound,
            "{storage:?}: a load peaked at {load_peak} bytes of heap; the graph rests at {resting} (bound {bound})"
        );
    }
}
