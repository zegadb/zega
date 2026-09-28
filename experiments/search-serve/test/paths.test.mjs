import assert from "node:assert/strict";
import test from "node:test";
import { RANKED_STEP, extractPath } from "../lib/paths.mjs";

const ENTITY = { qid: "Q205973", label: "Edmonton Oilers" };

function subgraph(over = {}) {
  return {
    entity: ENTITY,
    via: "label match",
    mentions: ["https://www.nhl.com/oilers/roster"],
    official: [{ prefix: "www.nhl.com/oilers/" }],
    links: new Map([
      ["https://www.nhl.com/oilers/roster", ["https://example.com/fan-post"]],
    ]),
    titles: new Map([
      ["https://www.nhl.com/oilers/roster", "Oilers roster"],
      ["https://example.com/fan-post", "A fan post"],
    ]),
    ...over,
  };
}

test("mentions edge produces entity -[mentions]- page", () => {
  const path = extractPath(subgraph(), "https://www.nhl.com/oilers/roster");
  assert.deepEqual(path, [
    { type: "entity", qid: "Q205973", label: "Edmonton Oilers", via: "label match" },
    { edge: "mentions" },
    { type: "page", url: "https://www.nhl.com/oilers/roster", title: "Oilers roster" },
    RANKED_STEP,
  ]);
});

test("official-source path carries the path prefix, not just the host", () => {
  const path = extractPath(
    subgraph({ mentions: [] }),
    "https://www.nhl.com/oilers/tickets",
  );
  assert.deepEqual(path[1], { edge: "official site", prefix: "www.nhl.com/oilers/" });
  assert.equal(path[2].url, "https://www.nhl.com/oilers/tickets");
});

test("a page on the same host but another team's prefix gets no official path", () => {
  // www.nhl.com is shared by every NHL team; host alone must never be enough.
  const path = extractPath(
    subgraph({ mentions: [], links: new Map() }),
    "https://www.nhl.com/flames/roster",
  );
  assert.deepEqual(path, [RANKED_STEP]);
});

test("link chain produces entity -[mentions]- page -[links to]- page", () => {
  const path = extractPath(subgraph(), "https://example.com/fan-post");
  assert.deepEqual(
    path.map((s) => s.edge ?? s.type ?? "step"),
    ["entity", "mentions", "page", "links to", "page", "step"],
  );
  assert.equal(path.at(-2).url, "https://example.com/fan-post");
});

test("a page with no graph relationship shows only the relevance step", () => {
  const path = extractPath(subgraph({ mentions: [], official: [], links: new Map() }), "https://example.com/fan");
  assert.deepEqual(path, [RANKED_STEP]);
});

test("transparency paths never expose scores (APS 29)", () => {
  for (const url of [
    "https://www.nhl.com/oilers/roster",
    "https://example.com/fan-post",
    "https://example.com/nowhere",
  ]) {
    const path = extractPath(subgraph(), url);
    assert.ok(!/score|bm25|weight/i.test(JSON.stringify(path)), `path for ${url} leaks scoring`);
    assert.deepEqual(path.at(-1), RANKED_STEP);
  }
});
