// Build the search graph from the intake artifacts (no new crawling) and load
// it into zega under experiments/search-serve/schema.zql.
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { ROOT, zql } from "./graph.mjs";
import { entityTerms } from "./rank.mjs";

const INTAKE = path.join(ROOT, "experiments/search-intake");
const SERVE = path.join(ROOT, "experiments/search-serve");
const LOAD_DIR = path.join(ROOT, ".tmp", "search-serve");
const MAX_SOURCE_BYTES = 1_800_000; // engine import limit is 2,000,000 per source

// Wikidata P31 labels for the kinds present in the intake seed set.
const KIND_LABELS = {
  Q4498974: "ice hockey team",
  Q15991290: "professional sports league",
  Q75179296: "ice hockey league",
  Q11422536: "international sport governing body",
  Q63364175: "ice hockey federation",
  Q31629: "type of sport",
  Q212434: "Olympic sport",
  Q216048: "team sport",
  Q13137940: "sport with racquet/stick/club",
};

const hostOf = (url) => new URL(url.includes("://") ? url : "https://" + url).hostname.toLowerCase();

function wordBoundaryMatch(term, haystack) {
  const escaped = term.toLowerCase().replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  return new RegExp(`(?<![\\w'])${escaped}(?![\\w'])`).test(haystack);
}

// Read the intake artifacts into the search-graph shape.
export function buildCorpus() {
  const rawEntities = JSON.parse(readFileSync(path.join(INTAKE, "entities.json"), "utf8"));
  const rawPages = JSON.parse(readFileSync(path.join(INTAKE, "pages.json"), "utf8"));
  const relations = JSON.parse(readFileSync(path.join(SERVE, "relations.json"), "utf8"));

  const entities = rawEntities.map((e) => ({
    qid: e.qid,
    label: e.label,
    aliases: e.aliases,
    kind: e.kind
      .split("|")
      .map((q) => KIND_LABELS[q] || q)
      .join(" | "),
    official_site: e.official_site,
    site_prefix: e.site_prefix || "",
  }));

  const captured = new Set(rawPages.map((p) => p.url));
  const pages = rawPages.map((p) => {
    const hay = `${p.title} ${p.text}`.toLowerCase();
    const mentions = entities
      .filter((e) => entityTerms(e).some((t) => t.length >= 3 && wordBoundaryMatch(t, hay)))
      .map((e) => e.qid);
    const neighbors = [
      ...new Set(
        String(p.outlinks || "")
          .split("\n")
          .filter((u) => captured.has(u) && u !== p.url),
      ),
    ];
    return {
      url: p.url,
      title: p.title,
      text: p.text,
      language: p.language,
      fetch_date: p.fetch_date,
      host: p.host,
      mentions,
      neighbors,
    };
  });

  const hosts = new Set(pages.map((p) => p.host));
  for (const e of entities) if (e.site_prefix) hosts.add(hostOf(e.site_prefix));
  const sites = [...hosts].sort().map((host) => ({ host }));
  const officialPrefixes = new Map(entities.filter((e) => e.site_prefix).map((e) => [e.qid, e.site_prefix]));

  return { sites, entities, pages, relations, officialPrefixes };
}

const z = (v) => JSON.stringify(v);

async function pool(items, size, fn) {
  const queue = [...items];
  await Promise.all(
    Array.from({ length: size }, async () => {
      while (queue.length) await fn(queue.shift());
    }),
  );
}

function writeChunks(name, rows) {
  mkdirSync(LOAD_DIR, { recursive: true });
  const files = [];
  let chunk = [];
  for (const row of rows) {
    const candidate = [...chunk, row];
    if (chunk.length && Buffer.byteLength(JSON.stringify(candidate)) > MAX_SOURCE_BYTES) {
      files.push(chunk);
      chunk = [row];
    } else {
      chunk = candidate;
    }
  }
  if (chunk.length) files.push(chunk);
  return files.map((rows, i) => {
    const rel = `.tmp/search-serve/${name}-${String(i).padStart(3, "0")}.json`;
    writeFileSync(path.join(ROOT, rel), JSON.stringify(rows));
    return { rel, count: rows.length, bytes: Buffer.byteLength(JSON.stringify(rows)) };
  });
}

// Load the corpus into the zega server at base. Returns load statistics.
export async function loadCorpus(base, corpus) {
  const t0 = Date.now();
  const chunks = [];
  const entityFiles = writeChunks("entities", corpus.entities);
  for (const f of entityFiles) {
    await zql(
      base,
      `mutation json ["${f.rel}"] { Entity(qid: $qid && label: $label && aliases: $aliases && kind: $kind && official_site: $official_site) { qid label aliases kind official_site } }`,
    );
    chunks.push({ file: f.rel, rows: f.count, bytes: f.bytes });
  }
  const siteFiles = writeChunks("sites", corpus.sites);
  for (const f of siteFiles) {
    await zql(base, `mutation json ["${f.rel}"] { Site(host: $host) { host } }`);
    chunks.push({ file: f.rel, rows: f.count, bytes: f.bytes });
  }
  const pageFiles = writeChunks(
    "pages",
    corpus.pages.map((p) => ({
      url: p.url,
      title: p.title,
      text: p.text,
      language: p.language,
      fetch_date: p.fetch_date,
    })),
  );
  for (const f of pageFiles) {
    await zql(
      base,
      `mutation json ["${f.rel}"] { Page(url: $url && title: $title && text: $text && language: $language && fetch_date: $fetch_date) { url title text language fetch_date } }`,
    );
    chunks.push({ file: f.rel, rows: f.count, bytes: f.bytes });
  }

  let edges = 0;
  const edgeOps = [];
  for (const p of corpus.pages) {
    edgeOps.push(`mutation { Page(url: ${z(p.url)}) { site -> link Site(host: ${z(p.host)}) } }`);
    for (const qid of p.mentions) {
      edgeOps.push(`mutation { Page(url: ${z(p.url)}) { mentions -> link Entity(qid: ${z(qid)}) { qid } } }`);
    }
    for (const nb of p.neighbors) {
      edgeOps.push(`mutation { Page(url: ${z(p.url)}) { linksTo -> link Page(url: ${z(nb)}) { url } } }`);
    }
  }
  for (const e of corpus.entities) {
    if (!e.site_prefix) continue;
    edgeOps.push(
      `mutation { Entity(qid: ${z(e.qid)}) { officialSiteOf -> link Site(host: ${z(hostOf(e.site_prefix))}) { &path_prefix: ${z(e.site_prefix)} } } }`,
    );
  }
  for (const r of corpus.relations) {
    const field = r.rel === "MEMBER_OF" ? "memberOf" : "sport";
    edgeOps.push(`mutation { Entity(qid: ${z(r.from)}) { ${field} -> link Entity(qid: ${z(r.to)}) { qid } } }`);
  }
  await pool(edgeOps, 16, async (op) => {
    await zql(base, op);
    edges += 1;
  });
  return {
    nodes: corpus.sites.length + corpus.entities.length + corpus.pages.length,
    edges,
    chunks,
    seconds: (Date.now() - t0) / 1000,
  };
}

// Read the ranking corpus back out of zega. The service never touches the
// intake files at query time; everything below is a ZQL result.
export async function fetchCorpus(base) {
  const pageRows = await zql(
    base,
    `{ Page { url title text language site -> Site { host } mentions -> Entity { qid } linksTo -> Page { url } } }`,
  );
  const entityRows = await zql(
    base,
    `{ Entity { qid label aliases kind official_site officialSiteOf -> Site { host &path_prefix } } }`,
  );
  const asList = (v) => (v == null ? [] : Array.isArray(v) ? v : [v]);
  const pages = pageRows.map((r) => ({
    url: r.url,
    title: r.title,
    text: r.text,
    language: r.language,
    host: r.site?.host ?? hostOf(r.url),
    mentions: asList(r.mentions).map((e) => e.qid),
    neighbors: asList(r.linksTo).map((p) => p.url),
  }));
  const entities = entityRows.map((r) => {
    const official = asList(r.officialSiteOf)[0];
    return {
      qid: r.qid,
      label: r.label,
      aliases: r.aliases,
      kind: r.kind,
      official_site: r.official_site,
      site_prefix: official?.path_prefix || "",
    };
  });
  const officialPrefixes = new Map(entities.filter((e) => e.site_prefix).map((e) => [e.qid, e.site_prefix]));
  return { pages, entities, officialPrefixes };
}

// Fetch the subgraph around one entity for transparency path extraction.
export async function fetchPathSubgraph(base, entity) {
  const rows = await zql(
    base,
    `{ Entity(qid: ${z(entity.qid)}) { label officialSiteOf -> Site { host &path_prefix } mentionedBy <- Page { url title linksTo -> Page { url title linksTo -> Page { url title } } } } }`,
  );
  const asList = (v) => (v == null ? [] : Array.isArray(v) ? v : [v]);
  const row = asList(rows)[0] || {};
  const mentions = asList(row.mentionedBy);
  const links = new Map();
  const titles = new Map();
  const addLink = (from, to) => {
    titles.set(from.url, from.title);
    titles.set(to.url, to.title);
    if (!links.has(from.url)) links.set(from.url, []);
    links.get(from.url).push(to.url);
  };
  for (const m of mentions) {
    titles.set(m.url, m.title);
    for (const nb of asList(m.linksTo)) {
      addLink(m, nb);
      for (const nb2 of asList(nb.linksTo)) addLink(nb, nb2);
    }
  }
  return {
    entity: { qid: entity.qid, label: row.label ?? entity.label },
    mentions: mentions.map((m) => m.url),
    official: asList(row.officialSiteOf).map((s) => ({ prefix: s.path_prefix })),
    links,
    titles,
  };
}
