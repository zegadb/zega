// ZQL client + zega server lifecycle for the search-serve experiment.
// Owns every process it starts and kills them by PID on close.
import { spawn } from "node:child_process";
import { mkdtempSync, readFileSync } from "node:fs";
import { createInterface } from "node:readline";
import path from "node:path";
import { fileURLToPath } from "node:url";

export const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../..");
export const SCHEMA = readFileSync(path.join(ROOT, "experiments/search-serve/schema.zql"), "utf8");

export async function zql(base, query) {
  const res = await fetch(base + "/zql", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ schema: SCHEMA, query }),
  });
  if (!res.ok) throw new Error(`zega HTTP ${res.status}: ${(await res.text()).slice(0, 2000)}`);
  const data = await res.json();
  if (!data.ok) throw new Error("zega ZQL error: " + JSON.stringify(data.error).slice(0, 2000));
  return data.result;
}

export function startZega() {
  const release = path.join(ROOT, ".target/release/zega");
  const debug = path.join(ROOT, ".target/debug/zega");
  let exe;
  try {
    readFileSync(release);
    exe = release;
  } catch {
    exe = debug;
  }
  const data = mkdtempSync(path.join(ROOT, ".tmp", "search-serve-db-"));
  const child = spawn(exe, ["start", "--data", data, "--port", "0"], {
    cwd: ROOT,
    stdio: ["ignore", "pipe", "inherit"],
  });
  const pid = child.pid;
  const base = new Promise((resolve, reject) => {
    const rl = createInterface({ input: child.stdout });
    const timer = setTimeout(() => reject(new Error("zega did not print its address")), 20000);
    rl.on("line", (line) => {
      const m = line.match(/http:\/\/127\.0\.0\.1:(\d+)/);
      if (m) {
        clearTimeout(timer);
        resolve(`http://127.0.0.1:${m[1]}`);
      }
    });
    child.on("exit", (code) => reject(new Error(`zega exited early (${code})`)));
  });
  return {
    pid,
    base,
    async close() {
      if (child.exitCode === null) {
        child.kill("SIGTERM");
        await new Promise((r) => child.once("exit", r));
      }
    },
  };
}

export async function waitHealthy(base) {
  const deadline = Date.now() + 20000;
  for (;;) {
    try {
      const res = await fetch(base + "/health");
      if (res.ok) return;
    } catch {}
    if (Date.now() > deadline) throw new Error("zega /health never came up");
    await new Promise((r) => setTimeout(r, 100));
  }
}
