// Build the Westeros sample from A Wiki of Ice and Fire (CC BY-SA 3.0; see
// ATTRIBUTION.md) and the cache westeros-fetch.mjs collects. Extracts facts
// only (no prose is copied): infobox fields such as Allegiance, Culture,
// Father/Mother, Spouse, Seat, Words, Date, Place and Commanders, read from
// each page's own structured infobox table. `x`/`y` place each location on
// the shared 0-1000 known-world grid (x west->east, y north->south): 67
// anchor places are copied verbatim from the paused westeros-map draft, the
// rest are placed by hand relative to them or fitted onto them per region
// from hand-reads off fan maps (theMountainGoat/Tear's speculative map; see
// ATTRIBUTION.md). Never traced from any official map or artwork.
//
// Usage: node scripts/westeros-fetch.mjs && node scripts/westeros-sample.mjs
import { readFile, writeFile, mkdir } from 'node:fs/promises';
import { resolve } from 'node:path';
import { format } from './format.mjs';

const DATA = resolve('scripts/westeros-data');
const CACHE = resolve('../.tmp/wiki-cache/raw');

// ---------- infobox parsing ----------

function stripTags(html) {
  return html
    .replace(/<br\s*\/?>/gi, '\n')
    .replace(/<sup[^>]*class="reference"[\s\S]*?<\/sup>/g, '')
    .replace(/<[^>]+>/g, '')
    .replace(/&#91;\d+&#93;/g, '')
    .replace(/&#160;/g, ' ')
    .replace(/&#32;/g, ' ')
    .replace(/&#39;/g, "'")
    .replace(/&amp;/g, '&')
    .replace(/&quot;/g, '"')
    .replace(/\s+/g, ' ')
    .trim();
}

function decodeTitle(raw) {
  return decodeURIComponent(raw).replaceAll('_', ' ');
}

// Every infobox row as { label, links: [{title, text}], text }. `links` keeps
// each internal wiki link's canonical page title, which is how names,
// houses, culture and places are cross-referenced rather than string-matched.
// Section headers ("Commanders", "Combatants") open a label with no value of
// their own, and the td-only rows after them append to it — that is how
// battle pages list their commanders.
function parseInfobox(html) {
  const markerIdx = html.indexOf('infobox');
  if (markerIdx === -1) return [];
  const tableStart = html.indexOf('<table', Math.max(0, markerIdx - 80));
  if (tableStart === -1) return [];
  let depth = 0, i = tableStart;
  while (i < html.length) {
    if (html.startsWith('<table', i)) { depth++; i += 6; }
    else if (html.startsWith('</table>', i)) { depth--; i += 8; if (depth === 0) break; }
    else i++;
  }
  const block = html.slice(tableStart, i);
  const rows = [...block.matchAll(/<tr[^>]*>([\s\S]*?)<\/tr>/g)].map((m) => m[1]);
  const fields = [];
  const rowLinks = (valueHtml) => [...valueHtml.matchAll(/<a href="\/index\.php\/([^"#]+)"[^>]*>([\s\S]*?)<\/a>/g)]
    .map((a) => ({ title: decodeTitle(a[1]), text: stripTags(a[2]) }))
    .filter((l) => !l.title.startsWith('File:') && !l.title.startsWith('Special:'));
  for (const row of rows) {
    const th = row.match(/<th[^>]*>([\s\S]*?)<\/th>/);
    const td = row.match(/<td[^>]*>([\s\S]*?)<\/td>/);
    if (th && td) {
      fields.push({ label: stripTags(th[1]), links: rowLinks(td[1]), text: stripTags(td[1]) });
    } else if (th) {
      fields.push({ label: stripTags(th[1]), links: [], text: '' });
    } else if (td && fields.length) {
      const last = fields[fields.length - 1];
      last.links.push(...rowLinks(td[1]));
      const text = stripTags(td[1]);
      last.text = last.text ? `${last.text} ${text}` : text;
    }
  }
  return fields;
}

const field = (fields, name) => fields.find((f) => f.label.toLowerCase() === name.toLowerCase());

// The whole infobox table as HTML, including nested tables: battle and
// wedding pages carry their commanders in <b>Commanders</b> sections of a
// table nested inside the infobox, which the row parser can't reach.
function infoboxBlock(html) {
  const markerIdx = html.indexOf('infobox');
  if (markerIdx === -1) return '';
  const tableStart = html.indexOf('<table', Math.max(0, markerIdx - 80));
  if (tableStart === -1) return '';
  let depth = 0, i = tableStart;
  while (i < html.length) {
    if (html.startsWith('<table', i)) { depth++; i += 6; }
    else if (html.startsWith('</table>', i)) { depth--; i += 8; if (depth === 0) break; }
    else i++;
  }
  return html.slice(tableStart, i);
}

// Internal links in one <b>-named section of the infobox (up to the next
// section header — a bold cell spanning the nested table — or the end of the
// enclosing table). Plain <b> labels inside the section don't end it.
function sectionLinks(block, name) {
  const m = block.match(new RegExp(`<b>\\s*${name}\\s*</b>`));
  if (!m) return [];
  const rest = block.slice(m.index + m[0].length);
  const end = rest.search(/<td[^>]*colspan[= ][^>]*>\s*<b>|<\/table>/);
  const seg = end === -1 ? rest : rest.slice(0, end);
  return [...seg.matchAll(/<a href="\/index\.php\/([^"#]+)"[^>]*>/g)]
    .map((a) => decodeTitle(a[1]))
    .filter((t) => !t.startsWith('File:') && !t.startsWith('Special:'));
}

const wikiCache = new Map();
async function loadPage(title) {
  if (wikiCache.has(title)) return wikiCache.get(title);
  const safe = title.replaceAll('/', '_');
  let html = null, found = false;
  try {
    html = await readFile(resolve(CACHE, `${safe}.html`), 'utf8');
    found = true;
  } catch { /* not cached: missing or not yet fetched */ }
  const fields = html ? parseInfobox(html) : [];
  const entry = { title, found, fields, block: html ? infoboxBlock(html) : '' };
  wikiCache.set(title, entry);
  return entry;
}

// The node's display name: the canonical page title, parenthetical
// disambiguators included, so two same-named people (the two Aemon
// Targaryens, the many Brandon Starks) stay distinct nodes.
const displayName = (title) => title.trim();
const sourcePath = (title) => `/index.php/${title.replaceAll(' ', '_')}`;

// ---------- load seed data ----------

const housesSeed = JSON.parse(await readFile(resolve(DATA, 'houses.json'), 'utf8'));
const locationsSeed = JSON.parse(await readFile(resolve(DATA, 'locations.json'), 'utf8'));
delete locationsSeed._comment;
const charactersSeed = JSON.parse(await readFile(resolve(DATA, 'characters.json'), 'utf8'));
const eventsSeed = JSON.parse(await readFile(resolve(DATA, 'events.json'), 'utf8'));

// Region node names are AWOIAF's own article titles: the Essos regions have
// no "The" on the wiki ('Free Cities', 'Jade Sea', 'Shadow Lands', 'Red
// Waste', 'Summer Isles'), unlike the Westeros regions that keep theirs.
const REGION_NAMES = [
  'The North', 'The Vale', 'The Riverlands', 'The Iron Islands', 'The Westerlands', 'The Reach',
  'The Stormlands', 'Dorne', 'The Crownlands', 'Beyond the Wall', 'Free Cities',
  "Slaver's Bay", 'The Dothraki Sea', 'Jade Sea', 'Valyria', 'Shadow Lands',
  'Red Waste', 'Ibben', 'Summer Isles', 'The Stepstones',
];

// ---------- locations (and the position frame) ----------

// The 67 anchor places, copied verbatim from the paused westeros-map draft's
// places.json ("known-world" frame: x west->east, y north->south, 0-1000).
// Every other location is placed relative to them.
const ANCHORS = {
  'Castle Black': [142, 287.5], 'The Shadow Tower': [122, 288], 'Eastwatch-by-the-Sea': [153, 289],
  'Hardhome': [159.5, 266], "Craster's Keep": [130, 274], 'Fist of the First Men': [124, 269],
  'Winterfell': [112, 344], 'Last Hearth': [146, 311], 'Karhold': [175, 323],
  'The Dreadfort': [149.5, 338], 'Deepwood Motte': [85, 325], 'Bear Island': [83, 305],
  "Torrhen's Square": [94, 357], 'White Harbor': [129, 380], 'Barrowton': [89, 376],
  'Moat Cailin': [114, 388], 'Greywater Watch': [110, 409],
  'The Twins': [103, 426.6], 'Seagard': [100, 435], 'Riverrun': [104, 462.5],
  'Harrenhal': [129, 470], 'Maidenpool': [151, 471],
  'The Eyrie': [147, 441], 'Gulltown': [182, 448], 'Runestone': [187, 444],
  'The Bloody Gate': [142, 449], 'Sisterton': [141, 401],
  'Pyke': [63, 451], 'Ten Towers': [75, 445],
  'Casterly Rock': [64, 491], 'Lannisport': [61, 496], 'The Golden Tooth': [87, 478],
  "King's Landing": [143, 500.7], 'Dragonstone': [177, 477], 'Duskendale': [150, 487.5],
  "Storm's End": [176, 531], 'Evenfall Hall': [185, 530], 'Summerhall': [140, 537.5],
  'Nightsong': [106, 556],
  'Highgarden': [86, 547], 'Oldtown': [65, 574], 'Bitterbridge': [111, 524], 'Horn Hill': [86, 557],
  'Sunspear': [182, 592], 'Yronwood': [128, 576], 'Starfall': [91.5, 586], 'Skyreach': [110, 579],
  'Braavos': [226, 415], 'Pentos': [228, 496], 'Myr': [245, 546], 'Tyrosh': [209, 553],
  'Lys': [234, 589], 'Volantis': [319, 595], 'Norvos': [277, 467], 'Qohor': [323, 496],
  'Lorath': [261.5, 425.3],
  'Vaes Dothrak': [462, 544],
  'Astapor': [462, 634], 'Yunkai': [468, 606], 'Meereen': [482, 594], 'Old Ghis': [469, 664],
  'Qarth': [625, 710], 'Valyria': [391, 682], 'Mantarys': [404, 609],
  'Asshai': [905, 680], 'Port of Ibben': [338, 278], 'Tall Trees Town': [94, 692],
};

// Old hand-reads (old_x/old_y) come from theMountainGoat/Tear's fan map and
// must be mapped onto the anchor frame. These per-region affine fits
// (x = a*ox + b*oy + c in `x`, y likewise in `y`) are the least-squares fits
// of the anchor places' old reads onto their anchor-frame coordinates,
// computed when the dataset was built (three or more non-collinear anchors
// per region; worst residuals ~10 grid units after dropping misfitting
// reads). Regions without enough anchors — the Westerlands, Iron Islands
// and Beyond the Wall — use the GLOBAL per-axis fit instead.
const REGION_FITS = {
  'The North': { x: [0.2427, -0.0249, 27.4217], y: [-0.0144, 0.5327, 217.2095] },
  'The Vale': { x: [0.2070, -0.0203, 1.9662], y: [-0.0064, 0.1945, 329.8357] },
  'The Riverlands': { x: [0.1044, 0.0195, 56.1418], y: [0.0012, 0.1745, 335.1882] },
  'The Crownlands': { x: [0.1608, 0.0137, 21.2913], y: [-0.0671, -0.0364, 566.5238] },
  'The Stormlands': { x: [0.1386, -0.1092, 118.0397], y: [-0.0752, -0.0429, 625.5889] },
  'The Reach': { x: [-0.1376, -0.2535, 328.7588], y: [-0.3288, -0.1117, 769.6531] },
  'Dorne': { x: [0.1844, -0.0092, 14.7926], y: [0.0063, -0.0489, 617.4846] },
};
const GLOBAL_FIT = { x: (o) => 0.1463 * o + 46.67, y: (o) => 0.3578 * o + 257.58 };
const applyFit = (fit, x, y) => fit
  ? [fit.x[0] * x + fit.x[1] * y + fit.x[2], fit.y[0] * x + fit.y[1] * y + fit.y[2]]
  : [GLOBAL_FIT.x(x), GLOBAL_FIT.y(y)];

// AWOIAF titles its places with a leading "The" ("The Trident", "The
// Dreadfort") where infobox links often drop it ("Trident", "Dreadfort" are
// redirects). A few seats are named for a nearby place the dataset holds
// under its shorter page title. Resolve a link target to a dataset location
// by exact title, alias, then the "The "-prefixed form.
const LOCATION_ALIASES = {
  'Castle Cerwyn': 'Cerwyn',
  'Barrow Hall': 'Barrowton',
  'Mormont Keep': 'Bear Island',
  'Pinkmaiden Castle': 'Pinkmaiden',
};
const findLocation = (title) => {
  if (!title) return null;
  if (locationNames.has(title)) return title;
  const alias = LOCATION_ALIASES[title];
  if (alias && locationNames.has(alias)) return alias;
  const withThe = `The ${title}`;
  return locationNames.has(withThe) ? withThe : null;
};

const locations = [];
for (const [name, entry] of Object.entries(locationsSeed)) {
  const page = await loadPage(name);
  if (!page.found) { console.warn(`[locations] skip (no source): ${name}`); continue; }
  let x, y;
  if (ANCHORS[name]) [x, y] = ANCHORS[name];
  else if (ANCHORS[`The ${name}`]) [x, y] = ANCHORS[`The ${name}`];
  else if (entry.x !== undefined && entry.old_x === undefined) [x, y] = [entry.x, entry.y];
  else if (entry.old_x !== undefined) {
    [x, y] = applyFit(REGION_FITS[entry.region], entry.old_x, entry.old_y);
  } else { console.warn(`[locations] skip (no position): ${name}`); continue; }
  locations.push({ name, kind: entry.kind, region: entry.region, x: Math.round(x), y: Math.round(y), source: sourcePath(name) });
}

// ---------- regions ----------

const regions = [];
for (const name of REGION_NAMES) {
  const page = await loadPage(name);
  regions.push({ name, source: page.found ? sourcePath(name) : '' });
}
const regions_ = regions.filter((r) => r.source); // only regions we could actually source
const regionNames = new Set(regions_.map((r) => r.name));

// The publishable place set: found and in a sourced region, declared before
// houses and events key off it, so a CSV `link` can never dangle against the
// westeros-locations.csv rows.
const places = locations.filter((l) => regionNames.has(l.region));
for (const l of locations) if (!regionNames.has(l.region)) console.warn(`[locations] ${l.name}: region ${l.region} not sourced`);
const locationNames = new Set(places.map((l) => l.name));

// ---------- houses ----------

const houses = [];
for (const [name, seed] of Object.entries(housesSeed)) {
  const page = await loadPage(name);
  if (!page.found) { console.warn(`[houses] skip (no source): ${name}`); continue; }
  const seatField = field(page.fields, 'Seat');
  const seat = seatField?.links.map((l) => l.title).map(findLocation).find(Boolean) || null;
  const wordsField = field(page.fields, 'Words');
  const words = wordsField ? wordsField.text.replace(/^"|"$/g, '').trim() : null;
  if (!regionNames.has(seed.region)) { console.warn(`[houses] ${name}: region ${seed.region} not sourced`); continue; }
  houses.push({ name, words, region: seed.region, seat, overlord: seed.great ? null : seed.overlord, source: sourcePath(name) });
}
const houseNames = new Set(houses.map((h) => h.name));

// ---------- characters ----------

const characterTitles = new Set(Object.entries(charactersSeed).flatMap(([, arr]) => arr));
// Kings and queens use a royal infobox with different labels ("Died in",
// "Royal House", "Queen"/"Consort", "Other Titles"); every field falls back
// across its spellings so a royal page parses as fully as a knight's.
const firstField = (fields, names) => names.map((n) => field(fields, n)).find(Boolean);
// Infobox links often use a redirect title ("Robert I Baratheon" -> the
// "Robert Baratheon" page); when a link target is not itself a dataset node,
// a regnal numeral is the usual reason, so try it once without.
const resolveCharacter = (title) => {
  if (!title) return null;
  if (characterTitles.has(title)) return title;
  const alias = title.replace(/ (?:I|II|III|IV|V|VI) /, ' ');
  return characterTitles.has(alias) ? alias : title;
};
const characters = [];
for (const title of characterTitles) {
  const page = await loadPage(title);
  if (!page.found) { console.warn(`[characters] skip (no source): ${title}`); continue; }
  const died = firstField(page.fields, ['Died', 'Died in']);
  const titleField = firstField(page.fields, ['Title', 'Other Titles']);
  const cultureField = field(page.fields, 'Culture');
  const allegiance = firstField(page.fields, ['Allegiance', 'Royal House']);
  const fatherField = field(page.fields, 'Father');
  const motherField = field(page.fields, 'Mother');
  const spouseField = firstField(page.fields, ['Spouse', 'Queen', 'Consort']);
  // The first allegiance house is the character's house; later links are
  // overlords and orders (Gregor Clegane is sworn to the Lannisters, but he
  // is not "of House Lannister"). Service allegiance is not membership:
  // Syrio Forel served the Starks as Arya's teacher, but he is Braavosi.
  const houseLinks = (() => {
    if (title === 'Syrio Forel') return [];
    const t = (allegiance?.links || []).map((l) => l.title).find((t) => houseNames.has(t));
    return t ? [t] : [];
  })();
  characters.push({
    name: displayName(title),
    title: title,
    alive: !died,
    titleText: titleField?.links[0]?.text || null,
    culture: cultureField?.links[0]?.text || null,
    houses: houseLinks,
    father: resolveCharacter(fatherField?.links[0]?.title || null),
    mother: resolveCharacter(motherField?.links[0]?.title || null),
    spouses: (spouseField?.links || []).map((l) => resolveCharacter(l.title)),
    source: sourcePath(title),
  });
}
const characterByTitle = new Map(characters.map((c) => [c.title, c]));
if (characterByTitle.size !== characters.length) throw new Error('duplicate character titles in seeds');
const namesSeen = new Set();
for (const c of characters) {
  if (namesSeen.has(c.name)) throw new Error(`character name collision: ${c.name}`);
  namesSeen.add(c.name);
}
const characterExists = (title) => characterByTitle.has(title);

// Parents for the principal families, where the wiki's own infoboxes are
// inconsistent about recording them (page values always win). Without these,
// the Stark and Lannister children would have no parents and no siblings.
const CURATED_PARENTS = [
  ['Robb Stark', 'Eddard Stark', 'Catelyn Stark'],
  ['Sansa Stark', 'Eddard Stark', 'Catelyn Stark'],
  ['Arya Stark', 'Eddard Stark', 'Catelyn Stark'],
  ['Bran Stark', 'Eddard Stark', 'Catelyn Stark'],
  ['Rickon Stark', 'Eddard Stark', 'Catelyn Stark'],
  ['Eddard Stark', 'Rickard Stark', 'Lyarra Stark'],
  ['Cersei Lannister', 'Tywin Lannister', 'Joanna Lannister'],
  ['Jaime Lannister', 'Tywin Lannister', 'Joanna Lannister'],
  ['Tyrion Lannister', 'Tywin Lannister', 'Joanna Lannister'],
  ['Joffrey Baratheon', 'Robert Baratheon', 'Cersei Lannister'],
  ['Tommen Baratheon', 'Robert Baratheon', 'Cersei Lannister'],
  ['Myrcella Baratheon', 'Robert Baratheon', 'Cersei Lannister'],
  ['Catelyn Stark', 'Hoster Tully', 'Minisa Tully'],
  ['Lysa Arryn', 'Hoster Tully', 'Minisa Tully'],
  ['Edmure Tully', 'Hoster Tully', 'Minisa Tully'],
  ['Robert Arryn', 'Jon Arryn', 'Lysa Arryn'],
  ['Margaery Tyrell', 'Mace Tyrell', 'Alerie Hightower'],
  ['Loras Tyrell', 'Mace Tyrell', 'Alerie Hightower'],
  ['Willas Tyrell', 'Mace Tyrell', 'Alerie Hightower'],
  ['Garlan Tyrell', 'Mace Tyrell', 'Alerie Hightower'],
  ['Theon Greyjoy', 'Balon Greyjoy', 'Alannys Harlaw'],
  ['Asha Greyjoy', 'Balon Greyjoy', 'Alannys Harlaw'],
  ['Maron Greyjoy', 'Balon Greyjoy', 'Alannys Harlaw'],
  ['Rodrik Greyjoy', 'Balon Greyjoy', 'Alannys Harlaw'],
  ['Arianne Martell', 'Doran Martell', null],
  ['Quentyn Martell', 'Doran Martell', null],
  ['Trystane Martell', 'Doran Martell', null],
  ['Rhaegar Targaryen', 'Aerys II Targaryen', 'Rhaella Targaryen'],
  ['Viserys Targaryen', 'Aerys II Targaryen', 'Rhaella Targaryen'],
  ['Daenerys Targaryen', 'Aerys II Targaryen', 'Rhaella Targaryen'],
  ['Ramsay Bolton', 'Roose Bolton', null],
  ['Domeric Bolton', 'Roose Bolton', null],
  ['Lyanna Mormont', 'Maege Mormont', null],
];
for (const [child, father, mother] of CURATED_PARENTS) {
  const c = characterByTitle.get(child);
  if (!c) continue;
  if (!c.father && father && characterExists(father)) c.father = father;
  if (!c.mother && mother && characterExists(mother)) c.mother = mother;
}

// ---------- events ----------

function extractYear(text) {
  // AWOIAF writes the conquest-era year as "299 AC" on newer pages and the
  // same era as "298AL" (Years after Aegon's Landing) on older ones.
  const m = text && text.match(/(\d{1,4})\s*A[CL]/);
  return m ? Number(m[1]) : null;
}

const events = [];
for (const [kind, titles] of Object.entries(eventsSeed)) {
  for (const title of titles) {
    const page = await loadPage(title);
    if (!page.found) { console.warn(`[events] skip (no source): ${title}`); continue; }
    const dateField = field(page.fields, 'Date');
    const placeField = field(page.fields, 'Place');
    const commanders = field(page.fields, 'Commanders');
    const place = placeField?.links.map((l) => l.title).map(findLocation).find(Boolean) || null;
    // Commanders: a plain field row where the template has one, else the
    // <b>Commanders</b> section of the nested battle table.
    const participants = [...new Set([
      ...(commanders?.links || []).map((l) => l.title),
      ...sectionLinks(page.block, 'Commanders'),
    ])].filter(characterExists);
    events.push({ name: title, kind, year: extractYear(dateField?.text), place, participants, source: sourcePath(title) });
  }
}

// ---------- derived relationships ----------

// Siblings: any two characters (in this dataset) sharing a recorded father or
// mother. Symmetric, so both orderings are emitted.
const siblingPairs = [];
for (const key of ['father', 'mother']) {
  const byParent = new Map();
  for (const c of characters) {
    if (!c[key] || !characterExists(c[key])) continue;
    if (!byParent.has(c[key])) byParent.set(c[key], []);
    byParent.get(c[key]).push(c.title);
  }
  for (const kids of byParent.values()) {
    for (let i = 0; i < kids.length; i++) for (let j = 0; j < kids.length; j++) if (i !== j) siblingPairs.push([kids[i], kids[j]]);
  }
}
const uniquePairs = (pairs) => [...new Map(pairs.map((p) => [p.join('\u0000'), p])).values()];
const siblings = uniquePairs(siblingPairs);

const spousePairs = uniquePairs(
  characters.flatMap((c) => c.spouses.filter(characterExists).flatMap((s) => [[c.title, s], [s, c.title]])),
);

// Killed by: hand-curated from the same character pages (each victim's own
// page names a killer AWOIAF rarely stores as a structured field). Book
// canon only — show-only deaths (Mance, Myrcella) are deliberately absent;
// each entry was checked by hand against the victim's page, and a sample is
// re-checked in browser/tests/westeros.spec.js.
const KILLED_BY = [
  ['Aerys II Targaryen', 'Jaime Lannister'],
  ['Tywin Lannister', 'Tyrion Lannister'],
  ['Jon Arryn', 'Lysa Arryn'],
  ['Eddard Stark', 'Ilyn Payne'],
  ['Viserys Targaryen', 'Khal Drogo'],
  ['Khal Drogo', 'Daenerys Targaryen'],
  ['Renly Baratheon', 'Stannis Baratheon'],
  ['Robb Stark', 'Roose Bolton'],
  ['Catelyn Stark', 'Raymund Frey'],
  ['Joffrey Baratheon', 'Olenna Tyrell'],
  ['Oberyn Martell', 'Gregor Clegane'],
  ['Craster', 'Dirk'],
  ['Qhorin Halfhand', 'Jon Snow'],
  ['Vargo Hoat', 'Gregor Clegane'],
  ['Amory Lorch', 'Vargo Hoat'],
  ['Polliver', 'Arya Stark'],
  ['Rorge', 'Brienne of Tarth'],
  ['Gregor Clegane', 'Oberyn Martell'],
];

const heldBy = [
  // (location, character, from AC, to AC) — the well-documented span of a
  // major seat's ruler, per that location's and character's own pages (the
  // 262/283/267 starts are quoted from the cached pages; later years are the
  // book timeline's convention, AC).
  ['Winterfell', 'Eddard Stark', 283, 298],
  ['Winterfell', 'Robb Stark', 298, 299],
  ['Winterfell', 'Ramsay Bolton', 299, 300],
  ["King's Landing", 'Aerys II Targaryen', 262, 283],
  ["King's Landing", 'Robert Baratheon', 283, 298],
  ["King's Landing", 'Joffrey Baratheon', 298, 300],
  ["King's Landing", 'Tommen Baratheon', 300, null],
  ['Casterly Rock', 'Tywin Lannister', 267, 300],
  ['Riverrun', 'Hoster Tully', null, 298],
  ['Riverrun', 'Edmure Tully', 298, 299],
  ['The Eyrie', 'Jon Arryn', null, 298],
  ['The Eyrie', 'Robert Arryn', 298, null],
  ['Highgarden', 'Mace Tyrell', null, null],
  ["Storm's End", 'Stannis Baratheon', 298, 299],
  ["Storm's End", 'Renly Baratheon', null, 298],
  ['Sunspear', 'Doran Martell', null, null],
  ['Pyke', 'Balon Greyjoy', null, 300],
  ['Pyke', 'Euron Greyjoy', 300, null],
  ['Dragonstone', 'Stannis Baratheon', null, 300],
].filter(([loc, name]) => locationNames.has(loc) && characterExists(name));

// ---------- csv / zql writers ----------

await mkdir('samples', { recursive: true });
// CSV cells keep ZQL's scalar inference (null, boolean, integer, decimal,
// otherwise string). An absent optional value is an EMPTY cell: the engine
// reads it as no value (queried back as JSON null). The bare token `null`
// would read as the four-letter STRING "null" — never write it.
const cell = (v) => (v === null || v === undefined ? '' : /[",\n]/.test(String(v)) ? `"${String(v).replaceAll('"', '""')}"` : String(v));
const csv = (header, rows) => [header, ...rows.map((r) => r.map(cell).join(','))].join('\n') + '\n';
const write = async (name, header, rows) => writeFile(`samples/${name}`, csv(header, rows));

await write('westeros-regions.csv', 'name,source', regions_.map((r) => [r.name, r.source]));
await write('westeros-locations.csv', 'name,kind,x,y,source,region',
  locations.filter((l) => regionNames.has(l.region)).map((l) => [l.name, l.kind, l.x, l.y, l.source, l.region]));
await write('westeros-houses.csv', 'name,words,source,region',
  houses.map((h) => [h.name, h.words, h.source, h.region]));
await write('westeros-houses-seat.csv', 'house,location', houses.filter((h) => h.seat).map((h) => [h.name, h.seat]));
await write('westeros-houses-sworn.csv', 'house,overlord', houses.filter((h) => h.overlord && houseNames.has(h.overlord)).map((h) => [h.name, h.overlord]));
await write('westeros-characters.csv', 'name,alive,title,culture,source',
  characters.map((c) => [c.name, c.alive, c.titleText, c.culture, c.source]));
await write('westeros-characters-member.csv', 'character,house',
  characters.flatMap((c) => c.houses.map((h) => [c.title, h])));
await write('westeros-characters-father.csv', 'child,father',
  characters.filter((c) => c.father && characterExists(c.father)).map((c) => [c.title, c.father]));
await write('westeros-characters-mother.csv', 'child,mother',
  characters.filter((c) => c.mother && characterExists(c.mother)).map((c) => [c.title, c.mother]));
await write('westeros-characters-spouse.csv', 'a,b', spousePairs);
await write('westeros-characters-sibling.csv', 'a,b', siblings);
await write('westeros-events.csv', 'name,kind,year,source', events.map((e) => [e.name, e.kind, e.year, e.source]));
await write('westeros-events-place.csv', 'event,location', events.filter((e) => e.place).map((e) => [e.name, e.place]));
await write('westeros-events-participant.csv', 'character,event', events.flatMap((e) => e.participants.map((p) => [p, e.name])));

const linkYear = ([, , from, to]) => {
  const props = [];
  if (from !== null && from !== undefined) props.push(`&from: ${from}`);
  if (to !== null && to !== undefined) props.push(`&to: ${to}`);
  return props.length ? ` { ${props.join(' ')} }` : '';
};

const heldByMutations = heldBy.map(([loc, name, from, to]) =>
  `mutation {\n  Character(name: ${JSON.stringify(name)}) {\n    heldSeat -> link Location(name: ${JSON.stringify(loc)})${linkYear([loc, name, from, to])}\n  }\n}`).join('\n\n');
const killedByMutations = KILLED_BY.filter(([v, k]) => characterExists(v) && characterExists(k)).map(([victim, killer]) =>
  `mutation {\n  Character(name: ${JSON.stringify(victim)}) {\n    killedBy -> link Character(name: ${JSON.stringify(killer)})\n  }\n}`).join('\n\n');

const zql = `// Westeros: an unofficial fan graph from A Song of Ice and Fire / Game of
// Thrones. Not affiliated with HBO or George R. R. Martin.
//
// Story facts (characters, houses, family, allegiances, seats, deaths,
// battles and weddings) are drawn from A Wiki of Ice and Fire
// (awoiaf.westeros.org), licensed CC BY-SA 3.0; every node's \`source\` field
// is the AWOIAF page path the facts came from. See ATTRIBUTION.md.
//
// Locations sit on zega's own 0-1000 grid (x west->east, y north->south) —
// our own geography, not traced from any official map, art or sigil.
// Positions are cross-checked against fan maps (see ATTRIBUTION.md); the
// anchor places are copied from the paused westeros-map draft and everything
// else is placed relative to them.
//
// Generated by browser/scripts/westeros-sample.mjs from
// browser/scripts/westeros-data/*.json and the Wayback Machine cache
// browser/scripts/westeros-fetch.mjs collects. Rerun both to refresh.
schema {
  type Region {
    name: String
    source: String
    locations: LOCATED_IN <- Location[]
  }

  type Location {
    name: String
    kind: String
    x: Int
    y: Int
    source: String
    region: LOCATED_IN -> Region
    seatOfHouse: SEAT <- House
    heldBy: HELD <- Character[] {
      from?: Int
      to?: Int
    }
    eventsHere: HAPPENED <- Event[]
  }

  type House {
    name: String
    words?: String
    source: String
    region: BASED_IN -> Region
    seat: SEAT -> Location
    swornTo: VASSAL -> House
    vassals: VASSAL <- House[]
    members: MEMBER <- Character[]
  }

  type Character {
    name: String
    alive: Bool
    title?: String
    culture?: String
    source: String
    memberOf: MEMBER -> House[]
    father: FATHER -> Character
    fatherOf: FATHER <- Character[]
    mother: MOTHER -> Character
    motherOf: MOTHER <- Character[]
    spouse: SPOUSE -> Character[]
    sibling: SIBLING -> Character[]
    killedBy: KILLED -> Character
    killed: KILLED <- Character[]
    heldSeat: HELD -> Location[] {
      from?: Int
      to?: Int
    }
    tookPartIn: PARTICIPANT -> Event[]
  }

  type Event {
    name: String
    kind: String
    year?: Int
    source: String
    happenedAt: HAPPENED -> Location
    participants: PARTICIPANT <- Character[]
  }

  display {
    table : Default
    graph
  }
}

unique {
  Region { name }
  Location { name }
  House { name }
  Character { name }
  Event { name }
}

mutation csv ["./samples/westeros-regions.csv"] { Region(name: $name && source: $source) { name } }

mutation csv ["./samples/westeros-locations.csv"] {
  Location(name: $name && kind: $kind && x: $x && y: $y && source: $source) { name }
}
mutation csv ["./samples/westeros-locations.csv"] {
  Location(name: $name) { region -> link Region(name: $region) }
}

mutation csv ["./samples/westeros-houses.csv"] {
  House(name: $name && words: $words && source: $source) { name }
}
mutation csv ["./samples/westeros-houses.csv"] {
  House(name: $name) { region -> link Region(name: $region) }
}
mutation csv ["./samples/westeros-houses-seat.csv"] {
  House(name: $house) { seat -> link Location(name: $location) }
}
mutation csv ["./samples/westeros-houses-sworn.csv"] {
  House(name: $house) { swornTo -> link House(name: $overlord) }
}

mutation csv ["./samples/westeros-characters.csv"] {
  Character(name: $name && alive: $alive && title: $title && culture: $culture && source: $source) { name }
}
mutation csv ["./samples/westeros-characters-member.csv"] {
  Character(name: $character) { memberOf -> link House(name: $house) }
}
mutation csv ["./samples/westeros-characters-father.csv"] {
  Character(name: $child) { father -> link Character(name: $father) }
}
mutation csv ["./samples/westeros-characters-mother.csv"] {
  Character(name: $child) { mother -> link Character(name: $mother) }
}
mutation csv ["./samples/westeros-characters-spouse.csv"] {
  Character(name: $a) { spouse -> link Character(name: $b) }
}
mutation csv ["./samples/westeros-characters-sibling.csv"] {
  Character(name: $a) { sibling -> link Character(name: $b) }
}

mutation csv ["./samples/westeros-events.csv"] {
  Event(name: $name && kind: $kind && year: $year && source: $source) { name }
}
mutation csv ["./samples/westeros-events-place.csv"] {
  Event(name: $event) { happenedAt -> link Location(name: $location) }
}
mutation csv ["./samples/westeros-events-participant.csv"] {
  Character(name: $character) { tookPartIn -> link Event(name: $event) }
}

// Killed by: hand-curated (see westeros-sample.mjs); AWOIAF rarely stores a
// killer as a structured infobox field, so these are checked by hand against
// each victim's own page.
${killedByMutations}

// Held by: the well-documented span of a major seat's ruler.
${heldByMutations}
`;

await writeFile('samples/westeros.zql', format(zql));

console.log(`Regions: ${regions_.length}`);
console.log(`Locations: ${locations.filter((l) => regionNames.has(l.region)).length}`);
console.log(`Houses: ${houses.length}`);
console.log(`Characters: ${characters.length}`);
console.log(`Events: ${events.length}`);
console.log(`memberOf: ${characters.flatMap((c) => c.houses).length}`);
console.log(`father: ${characters.filter((c) => c.father && characterExists(c.father)).length}`);
console.log(`mother: ${characters.filter((c) => c.mother && characterExists(c.mother)).length}`);
console.log(`spouse (directed): ${spousePairs.length}`);
console.log(`sibling (directed): ${siblings.length}`);
console.log(`sworn: ${houses.filter((h) => h.overlord && houseNames.has(h.overlord)).length}`);
console.log(`seat: ${houses.filter((h) => h.seat).length}`);
console.log(`killedBy: ${KILLED_BY.filter(([v, k]) => characterExists(v) && characterExists(k)).length}`);
console.log(`heldBy: ${heldBy.length}`);
console.log(`event participants: ${events.flatMap((e) => e.participants).length}`);
console.log(`event happenedAt: ${events.filter((e) => e.place).length}`);
