// Referee for search-serve: 25 queries, precision@1 and @3 per class, against
// a freshly loaded zega graph. No spike boost anywhere in the path.
import { readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { createSearchService, searchHandler } from "./serve.mjs";
import { ROOT } from "./lib/graph.mjs";

const matches = (url, expect) => {
  const path = url.replace(/^https?:\/\//, "").toLowerCase();
  return expect.some((e) => path.startsWith(e.toLowerCase()));
};

async function main() {
  const queries = JSON.parse(
    readFileSync(path.join(ROOT, "experiments/search-serve/queries.json"), "utf8"),
  );
  const service = await createSearchService();
  const handle = searchHandler(service);
  try {
    const rows = [];
    for (const { q, class: cls, expect } of queries) {
      const body = await handle(q);
      const urls = body.results.map((r) => r.url);
      const p1 = urls.length > 0 && matches(urls[0], expect);
      const p3 = urls.slice(0, 3).some((u) => matches(u, expect));
      rows.push({ q, class: cls, expect, top3: urls.slice(0, 3), p1, p3, transparency: body.transparency });
    }
    const classes = [...new Set(rows.map((r) => r.class))];
    const summary = {};
    for (const cls of classes) {
      const rs = rows.filter((r) => r.class === cls);
      summary[cls] = {
        n: rs.length,
        "p@1": rs.filter((r) => r.p1).length / rs.length,
        "p@3": rs.filter((r) => r.p3).length / rs.length,
      };
    }
    const out = { load: service.load, summary, rows };
    writeFileSync(
      path.join(ROOT, "experiments/search-serve/eval-results.json"),
      JSON.stringify(out, null, 2) + "\n",
    );
    console.log("| query | class | p@1 | p@3 | top-1 |");
    console.log("|---|---|---|---|---|");
    for (const r of rows) {
      console.log(
        `| ${r.q} | ${r.class} | ${r.p1 ? 1 : 0} | ${r.p3 ? 1 : 0} | ${r.top3[0] ?? "—"} |`,
      );
    }
    console.log("\nsummary:", JSON.stringify(summary));
  } finally {
    await service.zega.close();
  }
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
