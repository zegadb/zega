// Fetch the AWOIAF pages the Westeros sample needs, and cache the raw HTML
// for westeros-sample.mjs to parse. Rerunnable: pages already cached are
// skipped, so an interrupted run just picks up where it left off.
//
// The live wiki (awoiaf.westeros.org) sits behind a Cloudflare managed
// challenge that refuses a plain HTTP fetch (no JS engine to solve it), so
// this reads the same pages through the Wayback Machine — still A Wiki of
// Ice and Fire's own text, CC BY-SA 3.0
// (awoiaf.westeros.org/index.php/Special:Copyright); see ATTRIBUTION.md.
//
// Each page is probed at a handful of recent snapshot timestamps: Wayback
// redirects a timestamped URL to the nearest capture, so a page archived by
// any of the recent crawls comes back in one request. Pages the probes miss
// are asked of the light availability API next, and only the rest pay for a
// CDX lookup (the slow path: archive.org answers in seconds to tens of
// seconds under load and occasionally 503s, which the retry passes cover —
// a no-capture verdict needs three consistent CDX answers before it is
// believed). Requests stay one at a time with a delay between them: at most
// ~1 req/s to archive.org, and fewer requests overall than a CDX-first
// walk.
//
// Usage: node scripts/westeros-fetch.mjs [cacheDir] [--no-cdx] [--pending-only]
import { mkdir, readFile, writeFile, access } from 'node:fs/promises';
import { resolve } from 'node:path';

const UA = 'zega-westeros-dataset-build/1.0 (+https://zega.dev; fan-graph dataset build, not affiliated with HBO or George R. R. Martin; contact sfouad@gmail.com)';
const DELAY_MS = 1600;
// Probe timestamps spanning the archive's crawl eras (someone actively
// archives AWOIAF; Wayback redirects each probe to its nearest capture, so a
// page archived in any era answers one immediately). Pages with no capture
// near any era pay for a CDX lookup — the slow path: archive.org answers in
// seconds to tens of seconds under load.
const PROBES = ['20260901000000', '20260601183630', '20190414220233'];
const cacheArg = process.argv.slice(2).find((arg) => !arg.startsWith('--'));
// --no-cdx: probe-only sweep. Useful when archive.org's CDX endpoint is
// overloaded: pages archived near any known crawl era still resolve through
// the redirector, and the rest are dropped instead of blocking on CDX.
const ALLOW_CDX = !process.argv.includes('--no-cdx');
// --pending-only: skip titles already cached (or already judged unarchived)
// without the inter-request delay — a filesystem check makes no request, so
// pacing only applies between actual archive.org asks. Turns the full-list
// sweep (25 minutes of waiting on cache hits) into a quick pass over the
// outstanding titles.
const PENDING_ONLY = process.argv.includes('--pending-only');
const CACHE = resolve(cacheArg || '../.tmp/wiki-cache');
const RAW = resolve(CACHE, 'raw');
await mkdir(RAW, { recursive: true });

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function fetchWithRetry(url, tries = 8, timeoutMs = 30000) {
  let lastError;
  for (let i = 0; i < tries; i++) {
    try {
      const res = await fetch(url, { headers: { 'User-Agent': UA }, signal: AbortSignal.timeout(timeoutMs) });
      if (res.status === 200) return await res.text();
      if (res.status === 404) return null;
      lastError = new Error(`HTTP ${res.status}`);
      // 429/503 mean the archive asked us to slow down; back off harder than
      // a transient network error.
      if (res.status === 429 || res.status === 503) { await sleep(4000 * (i + 1)); continue; }
    } catch (error) { lastError = error; }
    await sleep(1500 * (i + 1));
  }
  throw lastError;
}

const cacheFile = (title) => resolve(RAW, `${title.replaceAll('/', '_')}.html`);
const metaFile = (title) => resolve(RAW, `${title.replaceAll('/', '_')}.meta.json`);
const known = async (title) => {
  try {
    await access(cacheFile(title));
    return true;
  } catch { /* fall through to the meta check */ }
  try {
    // A previous run already asked the archive about this page and was told
    // no snapshot exists; don't ask again.
    const meta = JSON.parse(await readFile(metaFile(title), 'utf8'));
    return meta.found === false;
  } catch {
    return false;
  }
};

// Wayback answers a probe with its own error page when nothing was ever
// archived for the URL, and MediaWiki renders an archived no-such-page with
// its stock sentence. Both markers are exact: loose phrases like "does not
// exist" appear in ordinary article prose.
const badContent = (html) =>
  /<title>Wayback Machine<\/title>/.test(html) ||
  /There is currently no text in this page/.test(html) ||
  html.length < 3000;

// MediaWiki's canonical page URL uses underscores; Wayback's capture lookup
// is literal about them (a %20 query 404s even when the underscore capture
// exists), and parentheses must be encoded for its redirector.
const wikiUrl = (title) => `https://awoiaf.westeros.org/index.php/${encodeURIComponent(title.replaceAll(' ', '_')).replaceAll('(', '%28').replaceAll(')', '%29')}`;

async function probe(title) {
  const original = wikiUrl(title);
  for (const ts of PROBES) {
    // Two tries per timestamp, gently paced: a burst of rapid retries is what
    // gets the archive's load balancer to 503 the next asks, and a genuinely
    // unarchived URL 404s persistently anyway.
    for (let attempt = 0; attempt < 2; attempt++) {
      const url = `https://web.archive.org/web/${ts}id_/${original}`;
      let res;
      try {
        res = await fetch(url, { headers: { 'User-Agent': UA }, signal: AbortSignal.timeout(30000), redirect: 'follow' });
      } catch { await sleep(2600); continue; }
      if (res.status !== 200) { await sleep(2600); continue; }
      const html = await res.text();
      // A 200 can still be Wayback's own error page or a MediaWiki
      // no-such-page under load; that is not proof the page is unarchived,
      // so try the next attempt before concluding anything.
      if (badContent(html)) { await sleep(2600); continue; }
      const m = res.url.match(/\/web\/(\d{14})id_\//);
      return { found: true, html, timestamp: m ? m[1] : ts, original };
    }
    await sleep(1100);
  }
  return null; // probes exhausted: ask CDX
}

async function latestSnapshot(title) {
  // One patient ask per try: CDX usually answers in 20-40 s under load, and a
  // hanging connection is cut at 90 s. A failure here marks the page 'error',
  // and a retry pass at the end of the run tries those again on a quiet
  // archive.
  const url = `https://web.archive.org/cdx/search/cdx?url=${wikiUrl(title).replace('https://', '')}&output=json&limit=-5&filter=statuscode:200`;
  // Under load the archive intermittently answers no-captures (an empty body,
  // or a header row the filters then leave without data rows) for URLs that
  // do have captures — a single ask can turn that hiccup into a permanent,
  // wrong "unarchived" verdict. A genuine no-match answers with the JSON
  // empty array "[]"; an EMPTY body is a degraded answer and must throw, not
  // count toward the no-capture streak. Require three consistent genuine
  // no-capture answers before believing one; a parse failure or a mixed
  // streak still throws, landing the title in the end-of-run error retry.
  let noCaptures = 0;
  for (let attempt = 0; attempt < 6; attempt++) {
    const text = await fetchWithRetry(url, 1, 90000);
    if (text !== null && text.trim() === '') throw new Error('CDX answered an empty body');
    if (text) {
      const rows = JSON.parse(text);
      if (Array.isArray(rows) && rows.length >= 2) return rows[rows.length - 1];
      if (!Array.isArray(rows)) throw new Error('CDX answered a non-JSON body');
    }
    noCaptures++;
    if (noCaptures >= 3) return null;
    await sleep(5000 * (attempt + 1));
  }
  throw new Error('CDX kept mixing empty and valid answers');
}

// The availability API is a lighter index than CDX and survives load that
// 504s CDX asks, but it is a coarse best-match lookup: an empty answer is
// NOT proof of "unarchived" — it drops to the authoritative CDX query below.
// A hit carries the closest capture's timestamp.
async function availabilitySnapshot(title) {
  const url = `https://archive.org/wayback/available?url=${wikiUrl(title).replace('https://', '')}&timestamp=${PROBES[0]}`;
  const text = await fetchWithRetry(url, 2, 30000);
  if (!text) return null;
  const closest = JSON.parse(text)?.archived_snapshots?.closest;
  return closest && closest.status === '200' && closest.available ? closest : null;
}

async function fetchOne(title, allowCdx = true) {
  if (await known(title)) return 'cached';
  try {
    const hit = await probe(title);
    if (hit && hit.found) {
      await writeFile(cacheFile(title), hit.html);
      await writeFile(metaFile(title), JSON.stringify({ title, found: true, timestamp: hit.timestamp, original: hit.original }));
      return 'fetched';
    }
    if (hit === null && allowCdx) {
      const av = await availabilitySnapshot(title);
      if (av) {
        const html = await fetchWithRetry(`https://web.archive.org/web/${av.timestamp}id_/${wikiUrl(title)}`);
        if (html && !badContent(html)) {
          await writeFile(cacheFile(title), html);
          await writeFile(metaFile(title), JSON.stringify({ title, found: true, timestamp: av.timestamp, original: wikiUrl(title) }));
          return 'fetched';
        }
      }
      const row = await latestSnapshot(title);
      if (!row) {
        await writeFile(metaFile(title), JSON.stringify({ title, found: false }));
        console.log(`[missing] ${title}`);
        return 'missing';
      }
      const [, timestamp, original] = row;
      const html = await fetchWithRetry(`https://web.archive.org/web/${timestamp}id_/${original}`);
      if (!html || badContent(html)) { console.error(`[error] ${title}: CDX snapshot failed validation`); return 'error'; }
      await writeFile(cacheFile(title), html);
      await writeFile(metaFile(title), JSON.stringify({ title, found: true, timestamp, original }));
      return 'fetched';
    }
    if (hit === null) {
      // Probes found nothing near any archived era; without a CDX ask the
      // page is treated as unarchived (dropped by the sample build).
      await writeFile(metaFile(title), JSON.stringify({ title, found: false }));
      return 'missing';
    }
    await writeFile(metaFile(title), JSON.stringify({ title, found: false }));
    return 'missing';
  } catch (error) {
    console.error(`[error] ${title}: ${error.message}`);
    return 'error';
  }
}

async function loadTitles() {
  const dataDir = resolve('scripts/westeros-data');
  const houses = JSON.parse(await readFile(resolve(dataDir, 'houses.json'), 'utf8'));
  const locations = JSON.parse(await readFile(resolve(dataDir, 'locations.json'), 'utf8'));
  delete locations._comment;
  const characters = JSON.parse(await readFile(resolve(dataDir, 'characters.json'), 'utf8'));
  const events = JSON.parse(await readFile(resolve(dataDir, 'events.json'), 'utf8'));
  const regions = [
    'The North', 'The Vale', 'The Riverlands', 'The Iron Islands', 'The Westerlands', 'The Reach',
    'The Stormlands', 'Dorne', 'The Crownlands', 'Beyond the Wall', 'Free Cities',
    "Slaver's Bay", 'The Dothraki Sea', 'Jade Sea', 'Valyria', 'Shadow Lands',
    'Red Waste', 'Ibben', 'Summer Isles', 'The Stepstones',
  ];
  const set = new Set();
  for (const t of Object.keys(houses)) set.add(t);
  for (const t of Object.keys(locations)) set.add(t);
  for (const arr of Object.values(characters)) for (const t of arr) set.add(t);
  for (const arr of Object.values(events)) for (const t of arr) set.add(t);
  for (const t of regions) set.add(t);
  return [...set].sort();
}

const titles = await loadTitles();
const outstanding = [];
for (const title of titles) if (!await known(title)) outstanding.push(title);
const ask = PENDING_ONLY ? outstanding : titles;
const stats = { cached: 0, fetched: 0, missing: 0, error: 0 };
const errored = [];
let done = 0;
for (const title of ask) {
  const status = await fetchOne(title, ALLOW_CDX);
  stats[status] = (stats[status] || 0) + 1;
  if (status === 'error') errored.push(title);
  if (status === 'fetched') console.log(`[fetched] ${title}`);
  done++;
  if (done % 5 === 0) console.log(`${done}/${ask.length} ${JSON.stringify(stats)}`);
  await sleep(DELAY_MS);
}
// Archive flakiness lands on arbitrary pages; every 'error' gets one clean
// retry now that the burst is over.
for (const title of errored) {
  const status = await fetchOne(title, ALLOW_CDX);
  stats.error -= 1;
  stats[status] = (stats[status] || 0) + 1;
  await sleep(DELAY_MS);
}
console.log('DONE', JSON.stringify({ total: ask.length, ...stats }));
