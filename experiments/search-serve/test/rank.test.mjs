import assert from "node:assert/strict";
import test from "node:test";
import {
  WEIGHTS,
  bm25Index,
  bm25Score,
  combineScore,
  editDistance,
  entitySignals,
  linkQuery,
  rank,
} from "../lib/rank.mjs";

const ENTITIES = [
  { qid: "Q205973", label: "Edmonton Oilers", aliases: "Alberta Oilers | Oilers" },
  { qid: "Q188143", label: "Montreal Canadiens", aliases: "Habs | Canadiens de Montréal" },
  { qid: "Q194126", label: "Calgary Flames", aliases: "Flames" },
  { qid: "Q203384", label: "Toronto Maple Leafs", aliases: "Maple Leafs" },
  { qid: "Q1215892", label: "National Hockey League", aliases: "" },
];

test("entity linking: exact label and alias matches", () => {
  assert.equal(linkQuery("Edmonton Oilers", ENTITIES)[0].entity.qid, "Q205973");
  assert.equal(linkQuery("habs", ENTITIES)[0].entity.qid, "Q188143");
  assert.equal(linkQuery("who are the oilers this season", ENTITIES)[0].entity.qid, "Q205973");
  // "leafs" is a word inside the multi-word label "Maple Leafs".
  assert.equal(linkQuery("leafs", ENTITIES)[0].entity.qid, "Q203384");
  // "nhl" is the derived acronym of "National Hockey League" (no such alias).
  assert.equal(linkQuery("nhl standings", ENTITIES)[0].entity.qid, "Q1215892");
  assert.equal(linkQuery("banana hammock", ENTITIES).length, 0);
});

test("entity linking: typo tolerance within bounded edit distance", () => {
  assert.equal(linkQuery("edmonton oiler", ENTITIES)[0].entity.qid, "Q205973");
  assert.equal(linkQuery("calgery flames", ENTITIES)[0].entity.qid, "Q194126");
  assert.equal(editDistance("calgery flames", "calgary flames"), 1);
  // Too far away to be a typo of anything.
  assert.equal(linkQuery("edmon oil zzz", ENTITIES).length, 0);
});

const PREFIXES = new Map([
  ["Q205973", "www.nhl.com/oilers/"],
  ["Q194126", "www.nhl.com/flames/"],
]);

test("official-source scoring is path-aware, not host-aware", () => {
  const linked = [{ entity: ENTITIES[0] }];
  const oilers = entitySignals(
    { url: "https://www.nhl.com/oilers/roster", mentions: [] },
    linked,
    PREFIXES,
    null,
  );
  assert.equal(oilers.official, true);
  // Same host, different team's path: not the Oilers' official source.
  const flames = entitySignals(
    { url: "https://www.nhl.com/flames/roster", mentions: [] },
    linked,
    PREFIXES,
    null,
  );
  assert.equal(flames.official, false);
  // Trailing-star prefix forms still match by their literal stem.
  const star = entitySignals(
    { url: "https://www.nhl.com/oilers/news/x", mentions: [] },
    linked,
    new Map([["Q205973", "www.nhl.com/oilers/*"]]),
    null,
  );
  assert.equal(star.official, true);
});

test("combineScore applies only the documented weight table", () => {
  const score = combineScore(2.5, { official: true, mention: true, hops: 2 });
  const expected =
    WEIGHTS.bm25 * 2.5 +
    WEIGHTS.officialSource +
    WEIGHTS.mention +
    WEIGHTS.proximity * WEIGHTS.proximityDecay;
  assert.equal(score, expected);
});

test("bm25 ranks a title match above a body-only mention", () => {
  // One occurrence each, and the title document is longer, so only the title
  // weight can put it ahead of the shorter body-only document.
  const pages = [
    { url: "a", title: "Oilers captain named", text: "season preview notes from camp today" },
    { url: "b", title: "Captain named today", text: "season preview oilers notes" },
  ];
  const index = bm25Index(pages);
  const a = bm25Score(index, index.docs[0], ["oilers"]);
  const b = bm25Score(index, index.docs[1], ["oilers"]);
  assert.ok(a > b, `title match ${a} should beat body stuffing ${b}`);
});

// The spike (PR #131) added +1000 whenever the query string equaled an entity
// label. That boost is gone: the same four score components apply to every
// query, so decorating the query string must not change any page's score.
test("no exact-entity-match boost survives in the ranker", () => {
  assert.deepEqual(
    Object.keys(WEIGHTS),
    ["bm25", "officialSource", "mention", "proximity", "proximityDecay"],
    "the weight table is the whole model; no boost key may appear",
  );
  const corpus = {
    entities: ENTITIES,
    officialPrefixes: PREFIXES,
    pages: [
      {
        url: "https://www.nhl.com/oilers/roster",
        title: "Oilers roster",
        text: "edmonton oilers roster page",
        host: "www.nhl.com",
        mentions: ["Q205973"],
        neighbors: [],
      },
      {
        url: "https://example.com/blog",
        title: "My blog",
        text: "oilers thoughts from a fan",
        host: "example.com",
        mentions: ["Q205973"],
        neighbors: [],
      },
    ],
  };
  const plain = rank("Edmonton Oilers", corpus).results.map((r) => [r.page.url, r.score]);
  for (const decorated of ["edmonton oilers", "EDMONTON OILERS", "Edmonton Oilers!", "Edmonton Oilers "]) {
    const again = rank(decorated, corpus).results.map((r) => [r.page.url, r.score]);
    assert.deepEqual(again, plain, `score must not depend on the raw query string (${decorated})`);
  }
});
