import { cp, mkdir, readFile, readdir, writeFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';
import postcss from 'postcss';
import { build } from 'esbuild';
import './check.mjs';

const browser = dirname(fileURLToPath(import.meta.url)).replace(/\/scripts$/, '');
const out = join(browser, 'dist-embed');
await mkdir(out, { recursive: true });
let shell = await readFile(join(browser, 'index.html'), 'utf8');
shell = shell.replace(/^[ \t]*<(?:button|a)\b[^>]*\bdata-site-only\b[^>]*>[\s\S]*?<\/(?:button|a)>\n/gm, '');
shell = shell.replace(/<head>[\s\S]*?<\/head>/, '').replace(/<script\b[^>]*>[\s\S]*?<\/script>/g, '').replace(/<\/?(?:html|body)\b[^>]*>/g, '');
await writeFile(join(out, 'shell.html'), shell.trim());

function scopeCss(css) {
  const root = postcss.parse(css);
  root.walkRules((rule) => {
    if (rule.parent?.type === 'atrule' && /keyframes$/i.test(rule.parent.name)) return;
    const scoped = rule.selectors.map((selector) => {
      const trimmed = selector.trim();
      if (/^(?:html|body|:root)(?=$|[.#:[\s])/.test(trimmed)) return trimmed.replace(/^(?:html|body|:root)/, '.zega-explorer');
      return `.zega-explorer ${trimmed}`;
    });
    const toolbarScoped = scoped.filter((selector) => selector !== '.zega-explorer').map((selector) => selector.replaceAll('.zega-explorer ', '.zega-explorer-toolbar '));
    rule.selector = [...scoped, ...toolbarScoped].join(', ');
    rule.walkDecls((decl) => {
      if (rule.selector === '.zega-explorer' && decl.prop === 'height' && decl.value === '100vh') decl.value = '100%';
    });
  });
  return root.toString();
}

const maplibre = await readFile(join(browser, 'vendor/maplibre-gl/maplibre-gl.css'), 'utf8');
const styles = await readFile(join(browser, 'style.css'), 'utf8');
await writeFile(join(out, 'embed.css'), `${scopeCss(maplibre)}\n${scopeCss(styles)}`);

await build({
  entryPoints: [join(browser, 'embed-entry.js')], outfile: join(out, 'embed.js'), bundle: true, platform: 'browser', format: 'esm', target: 'es2022', minify: true,
  plugins: [{ name: 'embed-scope', setup(esbuild) {
    esbuild.onLoad({ filter: /browser\/(?:repl|panes|editor|graph|map|globe|vector|table|csv|theme|backend|zql-edit|arcs|node-display)\.js$/ }, async ({ path }) => {
      let contents = await readFile(path, 'utf8');
      contents = contents
        .replaceAll('document.getElementById(', 'window.__zegaExplorerDocument.getElementById(')
        .replaceAll('document.querySelectorAll(', 'window.__zegaExplorerDocument.querySelectorAll(')
        .replaceAll('document.querySelector(', 'window.__zegaExplorerDocument.querySelector(')
        .replaceAll('document.documentElement', 'window.__zegaExplorerRoot')
        .replaceAll('document.body.appendChild(', 'window.__zegaExplorerRoot.appendChild(')
        .replaceAll('document.body.append(', 'window.__zegaExplorerRoot.append(')
        .replaceAll('document.addEventListener(', 'window.__zegaExplorerDocument.addEventListener(');
      return { contents, loader: 'js' };
    });
  } }],
});
const monaco = join(browser, 'node_modules/monaco-editor/min/vs');
await cp(monaco, join(out, 'monaco/vs'), { recursive: true });
for (const folder of ['vendor', 'data', 'fonts', 'samples']) await cp(join(browser, folder), join(out, folder), { recursive: true });
await cp(join(browser, 'pkg/zega_wasm_bg.wasm'), join(out, 'zega_wasm_bg.wasm'));
await cp(join(browser, '../LICENSE'), join(out, 'LICENSE'));

async function inventory(dir, base = '') {
  const files = {};
  for (const entry of await readdir(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    const name = base ? `${base}/${entry.name}` : entry.name;
    if (entry.isDirectory()) Object.assign(files, await inventory(path, name));
    else if (name !== 'artifact.json') files[name] = createHash('sha256').update(await readFile(path)).digest('hex');
  }
  return files;
}
const sourceCommit = execFileSync('git', ['rev-parse', 'HEAD'], { cwd: browser, encoding: 'utf8' }).trim();
const files = await inventory(out);
await writeFile(join(out, 'artifact.json'), JSON.stringify({ name: 'zega-explorer-embed', sourceCommit, files }, null, 2));
console.log(`Built the dashboard explorer module in browser/dist-embed/ (${Object.keys(files).length} files; Monaco is self-hosted).`);
