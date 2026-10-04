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
  if (options.database) {
    const response = await fetch(`${options.database.replace(/\/$/, '')}/schema`, { headers: { accept: 'application/json' } });
    const answer = await response.json();
    if (!response.ok || !answer.ok) throw new Error(answer.error ?? `Cannot read graph schema: HTTP ${response.status}`);
    localStorage.setItem('zega.v2.schema', answer.result.schema || 'type Person {\n  name: String\n}\n');
    const queryKey = options.graphId ? `zega.console.${options.graphId}.query` : 'zega.v2.query';
    localStorage.setItem('zega.v2.query', localStorage.getItem(queryKey) || '{\n  Person {\n    name\n  }\n}\n');
    localStorage.removeItem('zega.v2.sample');
  }
  await import('./repl.js');
  await import('./panes.js');
  if (options.database) {
    const push = options.chrome?.querySelector('#btn-push-schema') ?? root.querySelector('#btn-push-schema');
    const editor = window.monaco?.editor.getEditors().find((item) => item.getContainerDomNode().id === 'schema');
    const queryEditor = window.monaco?.editor.getEditors().find((item) => item.getContainerDomNode().id === 'query');
    const queryKey = options.graphId ? `zega.console.${options.graphId}.query` : 'zega.v2.query';
    queryEditor?.onDidChangeModelContent(() => localStorage.setItem(queryKey, queryEditor.getValue()));
    if (push && editor) {
      push.hidden = false;
      push.textContent = 'Push schema';
      push.onclick = async () => {
        if (!window.confirm('Push this schema to the graph? This updates its saved schema.')) return;
        const database = window.__zega;
        try {
          const answer = await database.pushSchema(editor.getValue());
          localStorage.setItem('zega.v2.schema', answer.schema);
          push.textContent = 'Schema pushed';
          setTimeout(() => { push.textContent = 'Push schema'; }, 1600);
        } catch (error) { window.alert(error instanceof Error ? error.message : String(error)); }
      };
    }
  }
  if (options.chrome) {
    const conn = options.chrome.querySelector('.conn');
    if (conn && options.label) conn.querySelector('#conn-label').textContent = options.label;
  }
  return { root, toolbar: options.chrome ?? root.querySelector('#topbar'), dispose() { root.replaceChildren(); options.chrome?.replaceChildren(); styles.remove(); root.classList.remove('zega-explorer'); } };
}

export { rootSelector };
