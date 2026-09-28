// Transparency path extraction (APS 29). The graph data always comes from
// zega through ZQL; this module only turns the fetched subgraph into the path
// shown to the user. Relationships are shown, scores never are; the ranking
// itself appears only as one vague "ranked by relevance" step.
//
// Path shapes, in preference order:
//   1. entity -[mentions]- page                      (MENTIONS edge)
//   2. entity -[official site <prefix>]- page        (OFFICIAL_SITE_OF, and the
//      page URL must really sit under the relationship's &path_prefix — the
//      site host alone is shared by every NHL team, so a host-only path would
//      be a lie)
//   3. entity -[mentions]- page -[links to]- page …  (BFS over LINKS_TO)

export const RANKED_STEP = { step: "ranked by relevance" };

// subgraph: {
//   entity: { qid, label },
//   via: "label match" | "alias" | "typo-tolerant match",
//   mentions: [url],               // pages with a MENTIONS edge to the entity
//   official: [{ prefix }],        // &path_prefix values on OFFICIAL_SITE_OF
//   links: Map(url -> [url]),      // LINKS_TO among fetched pages
//   titles: Map(url -> title),
// }
export function extractPath(subgraph, pageUrl) {
  const head = {
    type: "entity",
    qid: subgraph.entity.qid,
    label: subgraph.entity.label,
    via: subgraph.via,
  };
  const pageNode = () => ({
    type: "page",
    url: pageUrl,
    title: subgraph.titles.get(pageUrl) || null,
  });

  if (subgraph.mentions.includes(pageUrl)) {
    return [head, { edge: "mentions" }, pageNode(), RANKED_STEP];
  }

  const path = pageUrl.replace(/^https?:\/\//, "").toLowerCase();
  const prefix = subgraph.official
    .map((o) => o.prefix.toLowerCase().replace(/\*$/, ""))
    .filter((p) => p && path.startsWith(p))
    .sort((a, b) => b.length - a.length)[0];
  if (prefix) {
    return [head, { edge: "official site", prefix }, pageNode(), RANKED_STEP];
  }

  // BFS: entity -> mentioning page -> linksTo ... -> pageUrl
  const prev = new Map();
  const queue = [];
  for (const url of subgraph.mentions) {
    prev.set(url, null);
    queue.push(url);
  }
  while (queue.length) {
    const url = queue.shift();
    if (url === pageUrl) break;
    for (const nb of subgraph.links.get(url) || []) {
      if (!prev.has(nb)) {
        prev.set(nb, url);
        queue.push(nb);
      }
    }
  }
  if (prev.has(pageUrl)) {
    const chain = [];
    for (let u = pageUrl; u !== null; u = prev.get(u)) chain.unshift(u);
    const steps = [head, { edge: "mentions" }];
    chain.forEach((url, i) => {
      if (i > 0) steps.push({ edge: "links to" });
      steps.push({
        type: "page",
        url,
        title: subgraph.titles.get(url) || null,
      });
    });
    steps.push(RANKED_STEP);
    return steps;
  }

  // No graph relationship from the linked entity: the page is here on text
  // relevance alone, and the path says exactly that.
  return [RANKED_STEP];
}
