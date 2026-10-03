import { cp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import './check.mjs';

process.chdir(fileURLToPath(new URL('..', import.meta.url)));

// `--cli` builds the page the `zega` command serves (dist-cli/): the same
// source, without the website. The standalone site (dist/) keeps its links,
// sample buttons and samples; the CLI's page opens on its user's own database
// (or a cloud graph) and has none of them.
const cli = process.argv.includes('--cli');
const out = cli ? 'dist-cli' : 'dist';
// Samples are the site's examples; components/ is its component demo, which no explorer module imports.
const SITE_ONLY_ASSETS = new Set(['samples', 'components']);

await rm(out, { recursive: true, force: true });
await mkdir(out);
// Explicit deploy inputs keep repository metadata, tooling and local files out.
for (const path of JSON.parse(await readFile('assets.json', 'utf8'))) {
  if (cli && SITE_ONLY_ASSETS.has(path)) continue;
  await cp(path, `${out}/${path}`, { recursive: true });
}
await cp('../LICENSE', `${out}/LICENSE`);

if (cli) {
  let html = await readFile('index.html', 'utf8');
  // An element marked data-site-only is a button or link to the website or a sample: gone.
  const before = html;
  html = html.replace(/^[ \t]*<(button|a)\b[^>]*\bdata-site-only\b[^>]*>[\s\S]*?<\/\1>\n/gm, '');
  if (html === before) throw new Error('index.html has no data-site-only element to remove: the CLI build would be the site');
  // The brand is a link home on the site; here it is only the name.
  html = html.replace(/<a class="brand"[^>]*\bdata-site-link\b[^>]*>([\s\S]*?)<\/a>/, '<span class="brand" role="img" aria-label="zega">$1</span>');
  // The dialog's note names the site's api host; the CLI passes requests on itself.
  html = html.replace(/(<p class="remote-note">)[^<]*(<\/p>)/, "$1A Zega Cloud graph, by its id and a graph key (zk_…). The key stays in this page's memory: it is never saved, and a reload or Disconnect forgets it. The zega command passes your requests on to the graph and keeps nothing.$2");
  for (const left of ['zega.dev', 'data-site', 'btn-flights', 'btn-tickets', 'btn-cities', 'btn-westeros', 'btn-calgary', 'btn-seed']) {
    if (html.includes(left)) throw new Error(`the CLI page still mentions ${left}`);
  }
  await writeFile(`${out}/index.html`, html);
  console.log('Built the CLI explorer in dist-cli/ (no website links, no samples).');
} else {
  console.log('Built static explorer in dist/ using the vendored wasm (no engine checkout required).');
}
