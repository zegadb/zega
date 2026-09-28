// Entity linking, BM25, and entity-signal scoring for the search-serve
// experiment. Everything is pure functions over plain data so the unit tests
// exercise the same code the service and the referee run.

// ---------------------------------------------------------------------------
// Weights. The whole ranking model is this table plus BM25 (k1, b below).
// There is deliberately no exact-query-match boost: the spike
// (experiments/search-intake, PR #131) scored +1000 when the query string
// equaled an entity label; here a query only ever reaches a page through the
// same four components, whatever the query looks like.
//
//   bm25            text relevance over title + body, title counted 3x
//   officialSource  page URL sits under the linked entity's official path
//                   prefix (P856, path-aware: www.nhl.com/oilers/, not a host)
//   mention         the page has a MENTIONS edge to the linked entity
//   proximity       graph hops from the linked entity, x proximityDecay^(hops-1)
//
// officialSource > mention > proximity: being the entity's own site is the
// strongest statement a page can make, a mention is weaker, and link distance
// fades fast.
export const WEIGHTS = {
  bm25: 1.0,
  officialSource: 8.0,
  mention: 3.0,
  proximity: 2.0,
  proximityDecay: 0.5,
};

export const BM25 = { k1: 1.2, b: 0.75, titleWeight: 3.0 };

// ---------------------------------------------------------------------------
export function tokenize(text) {
  const out = [];
  for (const m of String(text).toLowerCase().matchAll(/[\w'-]+/g)) {
    if (m[0].length >= 2) out.push(m[0]);
  }
  return out;
}

export function entityTerms(entity) {
  const terms = [entity.label];
  for (const a of String(entity.aliases || "").split(" | ")) {
    if (a.trim()) terms.push(a.trim());
  }
  // Derived acronym for multi-word labels, so "nhl" finds the National Hockey
  // League even when Wikidata lists no such alias.
  const words = String(entity.label).toLowerCase().match(/[\w'-]+/g) || [];
  const acronym = words
    .filter((w) => w.length >= 3)
    .map((w) => w[0])
    .join("");
  if (acronym.length >= 3 && !terms.some((t) => t.toLowerCase() === acronym)) {
    terms.push(acronym);
  }
  return terms;
}

function wordBoundaryMatch(term, haystack) {
  const escaped = term.toLowerCase().replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  return new RegExp(`(?<![\\w'])${escaped}(?![\\w'])`).test(haystack);
}

// Damerau-Levenshtein distance, enough for single-typo tolerance.
export function editDistance(a, b) {
  const m = a.length;
  const n = b.length;
  const d = Array.from({ length: m + 1 }, (_, i) => [i, ...Array(n).fill(0)]);
  for (let j = 0; j <= n; j++) d[0][j] = j;
  for (let i = 1; i <= m; i++) {
    for (let j = 1; j <= n; j++) {
      const cost = a[i - 1] === b[j - 1] ? 0 : 1;
      d[i][j] = Math.min(d[i - 1][j] + 1, d[i][j - 1] + 1, d[i - 1][j - 1] + cost);
      if (i > 1 && j > 1 && a[i - 1] === b[j - 2] && a[i - 2] === b[j - 1]) {
        d[i][j] = Math.min(d[i][j], d[i - 2][j - 2] + cost);
      }
    }
  }
  return d[m][n];
}

function typoTolerance(term) {
  return term.length > 8 ? 2 : 1;
}

// Link a query to entities: exact word-boundary label/alias matches first
// (either direction: the query names the entity, or the query is a word inside
// a multi-word label, as in "leafs"), then whole-query fuzzy matches with
// bounded edit distance. Returns best-first [{ entity, via, strength }].
export function linkQuery(query, entities) {
  const q = String(query).toLowerCase().trim();
  const found = new Map();
  for (const entity of entities) {
    for (const term of entityTerms(entity)) {
      const t = term.toLowerCase();
      if (t.length < 2) continue;
      if (wordBoundaryMatch(t, q) || (t.includes(" ") && wordBoundaryMatch(q, t))) {
        const strength = t.length;
        if (!found.has(entity.qid) || found.get(entity.qid).strength < strength) {
          found.set(entity.qid, { entity, via: term, strength });
        }
      }
    }
  }
  if (found.size === 0 && q.length >= 4) {
    for (const entity of entities) {
      for (const term of entityTerms(entity)) {
        const t = term.toLowerCase();
        if (Math.abs(t.length - q.length) > typoTolerance(t)) continue;
        if (editDistance(q, t) <= typoTolerance(t)) {
          const strength = t.length - 0.5;
          if (!found.has(entity.qid) || found.get(entity.qid).strength < strength) {
            found.set(entity.qid, { entity, via: `~${term}`, strength });
          }
        }
      }
    }
  }
  return [...found.values()].sort((a, b) => b.strength - a.strength);
}

// ---------------------------------------------------------------------------
// BM25 over title + body. Title tokens count titleWeight times.
export function bm25Index(pages) {
  const docs = pages.map((p) => {
    const title = tokenize(p.title);
    const body = tokenize(p.text);
    const tf = new Map();
    for (const t of title) tf.set(t, (tf.get(t) || 0) + BM25.titleWeight);
    for (const t of body) tf.set(t, (tf.get(t) || 0) + 1);
    return { url: p.url, tf, len: title.length * BM25.titleWeight + body.length };
  });
  const avgdl = docs.reduce((s, d) => s + d.len, 0) / Math.max(docs.length, 1);
  const df = new Map();
  for (const d of docs) for (const t of d.tf.keys()) df.set(t, (df.get(t) || 0) + 1);
  return { docs, avgdl, df, n: docs.length };
}

export function bm25Score(index, doc, queryTerms) {
  let score = 0;
  for (const t of queryTerms) {
    const tf = doc.tf.get(t);
    if (!tf) continue;
    const df = index.df.get(t) || 0;
    const idf = Math.log(1 + (index.n - df + 0.5) / (df + 0.5));
    score +=
      (idf * tf * (BM25.k1 + 1)) /
      (tf + BM25.k1 * (1 - BM25.b + (BM25.b * doc.len) / index.avgdl));
  }
  return score;
}

// ---------------------------------------------------------------------------
// Entity signals for one page against the query's linked entities.
// page: { url, mentions: [qid] }, proximity: Map(qid -> hops to this page).
export function entitySignals(page, linked, officialPrefixes, proximity) {
  let best = { official: false, mention: false, hops: null, entity: null };
  const pagePath = page.url.replace(/^https?:\/\//, "").toLowerCase();
  for (const { entity } of linked) {
    const prefix = (officialPrefixes.get(entity.qid) || "").toLowerCase();
    const official = Boolean(prefix) && pagePath.startsWith(prefix.replace(/\*$/, ""));
    const mention = page.mentions.includes(entity.qid);
    const hops = proximity?.get(entity.qid)?.get(page.url) ?? (mention ? 1 : null);
    if (official || mention || hops !== null) {
      const rank = (official ? 4 : 0) + (mention ? 2 : 0) + (hops !== null ? 1 / hops : 0);
      const bestRank =
        (best.official ? 4 : 0) + (best.mention ? 2 : 0) + (best.hops !== null ? 1 / best.hops : 0);
      if (rank > bestRank) best = { official, mention, hops, entity };
    }
  }
  return best;
}

export function combineScore(bm25, signals) {
  let score = WEIGHTS.bm25 * bm25;
  if (signals.official) score += WEIGHTS.officialSource;
  if (signals.mention) score += WEIGHTS.mention;
  if (signals.hops !== null) {
    score += WEIGHTS.proximity * Math.pow(WEIGHTS.proximityDecay, signals.hops - 1);
  }
  return score;
}

// Graph proximity: BFS from each linked entity over MENTIONS (entity->page)
// and LINKS_TO (page->page, followed both ways). Returns Map(qid -> Map(url -> hops)).
export function proximityMaps(linked, pagesByUrl, mentionEdges, maxHops = 3) {
  const out = new Map();
  for (const { entity } of linked) {
    const hops = new Map();
    let frontier = [];
    for (const [url, qids] of mentionEdges) {
      if (qids.includes(entity.qid)) {
        hops.set(url, 1);
        frontier.push(url);
      }
    }
    let depth = 1;
    while (frontier.length && depth < maxHops) {
      depth += 1;
      const next = [];
      for (const url of frontier) {
        const page = pagesByUrl.get(url);
        if (!page) continue;
        for (const nb of page.neighbors) {
          if (!hops.has(nb)) {
            hops.set(nb, depth);
            next.push(nb);
          }
        }
      }
      frontier = next;
    }
    out.set(entity.qid, hops);
  }
  return out;
}

// Full ranking pipeline. corpus: { pages, entities, officialPrefixes }.
// Each page: { url, title, text, host, mentions: [qid], neighbors: [url] }.
export function rank(query, corpus) {
  const linked = linkQuery(query, corpus.entities);
  const queryTerms = [...new Set(tokenize(query).filter((t) => t.length >= 3))];
  const index = bm25Index(corpus.pages);
  const pagesByUrl = new Map(corpus.pages.map((p) => [p.url, p]));
  const mentionEdges = new Map(corpus.pages.map((p) => [p.url, p.mentions]));
  const proximity = proximityMaps(linked, pagesByUrl, mentionEdges);
  const scored = [];
  for (let i = 0; i < corpus.pages.length; i++) {
    const page = corpus.pages[i];
    const bm25 = bm25Score(index, index.docs[i], queryTerms);
    const signals = entitySignals(page, linked, corpus.officialPrefixes, proximity);
    const score = combineScore(bm25, signals);
    if (score > 0) scored.push({ page, score, signals });
  }
  scored.sort((a, b) => b.score - a.score || (a.page.url < b.page.url ? -1 : 1));
  return { query, linked, results: scored.slice(0, 10) };
}
