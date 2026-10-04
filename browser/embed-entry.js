const rootSelector = (selector) => window.__zegaExplorerRoot.querySelector(selector) ?? window.__zegaExplorerChrome?.querySelector(selector) ?? null;

export async function mountExplorer(root, options = {}) {
  if (!(root instanceof HTMLElement)) throw new TypeError('mountExplorer needs an HTMLElement root');
  root.classList.add('zega-explorer');
  window.__zegaExplorerRoot = root;
  window.__zegaExplorerChrome = options.chrome ?? null;
  const assetBase = (options.assetBase ?? '/explorer').replace(/\/$/, '');
  window.__zegaExplorerOptions = {
    ...options,
    assetBase,
    monacoBase: `${assetBase}/monaco`,
    wasmUrl: `${assetBase}/zega_wasm_bg.wasm`,
  };
  const response = await fetch(`${assetBase}/shell.html`);
  if (!response.ok) throw new Error(`Cannot load explorer shell: HTTP ${response.status}`);
  root.innerHTML = await response.text();
  window.__zegaExplorerDocument = {
    getElementById(id) { return root.querySelector(`#${CSS.escape(id)}`) ?? options.chrome?.querySelector(`#${CSS.escape(id)}`) ?? null; },
    querySelector: rootSelector,
    querySelectorAll(selector) { return [...root.querySelectorAll(selector), ...(options.chrome ? options.chrome.querySelectorAll(selector) : [])]; },
    addEventListener(type, handler, capture) { root.addEventListener(type, handler, capture); options.chrome?.addEventListener(type, handler, capture); },
  };
  const toolbar = root.querySelector('#topbar');
  if (options.chrome && toolbar) {
    toolbar.querySelector('.brand')?.remove();
    options.chrome.replaceChildren(toolbar);
    options.chrome.classList.add('zega-explorer-toolbar');
  }
  const styles = document.createElement('link');
  styles.rel = 'stylesheet';
  styles.href = `${assetBase}/embed.css`;
  document.head.append(styles);
  const loader = document.createElement('script');
  loader.src = `${assetBase}/monaco/vs/loader.js`;
  await new Promise((resolve, reject) => { loader.onload = resolve; loader.onerror = reject; document.head.append(loader); });
  await import('./repl.js');
  await import('./panes.js');
  if (options.chrome) {
    const conn = options.chrome.querySelector('.conn');
    if (conn && options.label) conn.querySelector('#conn-label').textContent = options.label;
  }
  return { root, toolbar: options.chrome ?? root.querySelector('#topbar'), dispose() { root.replaceChildren(); options.chrome?.replaceChildren(); styles.remove(); root.classList.remove('zega-explorer'); } };
}

export { rootSelector };
