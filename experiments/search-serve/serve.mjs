// Tiny local search service: GET /search?q= returns ranked results plus the
// real graph path from the query's linked entity to each page (APS 29).
// Ranking runs in this experiment; every graph fact comes from zega via ZQL.
import { createServer } from "node:http";
import { buildCorpus, fetchCorpus, fetchPathSubgraph, loadCorpus } from "./lib/corpus.mjs";
import { startZega, waitHealthy } from "./lib/graph.mjs";
import { extractPath } from "./lib/paths.mjs";
import { rank } from "./lib/rank.mjs";

function snippet(text, query) {
  const lower = text.toLowerCase();
  const terms = query.toLowerCase().match(/[\w'-]+/g) || [];
  let at = 0;
  for (const t of terms) {
    const i = lower.indexOf(t);
    if (i >= 0) {
      at = i;
      break;
    }
  }
  const start = Math.max(0, at - 60);
  return text.slice(start, start + 180).trim();
}

export async function createSearchService() {
  const zega = startZega();
  const base = await zega.base;
  await waitHealthy(base);
  const load = await loadCorpus(base, buildCorpus());
  const corpus = await fetchCorpus(base);
  return { zega, base, corpus, load };
}

export function searchHandler(service) {
  return async (q) => {
    const { linked, results } = rank(q, service.corpus);
    const viaOf = (link) =>
      link.via.startsWith("~")
        ? "typo-tolerant match"
        : link.via.toLowerCase() === link.entity.label.toLowerCase()
          ? "label match"
          : "alias match";
    const subgraphs = new Map();
    for (const link of linked.slice(0, 3)) {
      subgraphs.set(link.entity.qid, {
        ...(await fetchPathSubgraph(service.base, link.entity)),
        via: viaOf(link),
      });
    }
    const transparency = {};
    for (const { page, signals } of results) {
      const qid = signals.entity?.qid ?? linked[0]?.entity.qid;
      const subgraph = qid ? subgraphs.get(qid) : null;
      transparency[page.url] = subgraph
        ? extractPath(subgraph, page.url)
        : [{ step: "ranked by relevance" }];
    }
    return {
      results: results.map(({ page }) => ({
        url: page.url,
        title: page.title,
        snippet: snippet(page.text, q),
        site: page.host,
      })),
      transparency,
    };
  };
}

async function main() {
  const portArg = process.argv.indexOf("--port");
  const port = portArg >= 0 ? Number(process.argv[portArg + 1]) : 0;
  const service = await createSearchService();
  const handle = searchHandler(service);
  const server = createServer(async (req, res) => {
    const url = new URL(req.url, "http://localhost");
    if (url.pathname !== "/search") {
      res.writeHead(404, { "Content-Type": "application/json" });
      res.end(JSON.stringify({ error: "GET /search?q=" }));
      return;
    }
    const q = url.searchParams.get("q") || "";
    try {
      const body = await handle(q);
      res.writeHead(200, { "Content-Type": "application/json" });
      res.end(JSON.stringify(body));
    } catch (err) {
      res.writeHead(500, { "Content-Type": "application/json" });
      res.end(JSON.stringify({ error: String(err) }));
    }
  });
  server.listen(port, "127.0.0.1", () => {
    console.log(`search-serve listening on http://127.0.0.1:${server.address().port}/search?q=`);
  });
  const shutdown = async () => {
    server.close();
    await service.zega.close();
    process.exit(0);
  };
  process.on("SIGINT", shutdown);
  process.on("SIGTERM", shutdown);
}

if (process.argv[1] && process.argv[1].endsWith("serve.mjs")) {
  main().catch((err) => {
    console.error(err);
    process.exit(1);
  });
}
