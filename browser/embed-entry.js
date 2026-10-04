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
    window.__zegaExplorerPushedSchema = answer.result.schema ?? '';
    localStorage.setItem('zega.v2.schema', answer.result.schema || 'type Person {\n  name: String\n}\n');
    const queryKey = options.graphId ? `zega.console.${options.graphId}.query` : 'zega.v2.query';
    localStorage.setItem('zega.v2.query', localStorage.getItem(queryKey) || '');
    localStorage.removeItem('zega.v2.sample');
  }
  await import('./repl.js');
  await import('./panes.js');
  const syncTheme = () => {
    const theme = root.dataset.theme;
    if (theme) document.documentElement.dataset.theme = theme;
  };
  const themeObserver = new MutationObserver(syncTheme);
  themeObserver.observe(root, { attributes: true, attributeFilter: ['data-theme'] });
  syncTheme();
  if (options.database) {
    const push = options.chrome?.querySelector('#btn-push-schema') ?? root.querySelector('#btn-push-schema');
    const note = options.chrome?.querySelector('#push-note') ?? root.querySelector('#push-note');
    const editor = window.monaco?.editor.getEditors().find((item) => item.getContainerDomNode().id === 'schema');
    const queryEditor = window.monaco?.editor.getEditors().find((item) => item.getContainerDomNode().id === 'query');
    const queryKey = options.graphId ? `zega.console.${options.graphId}.query` : 'zega.v2.query';
    queryEditor?.onDidChangeModelContent(() => localStorage.setItem(queryKey, queryEditor.getValue()));
    if (push && editor) {
      push.hidden = false;
      push.textContent = 'Push schema';
      let pushedSchema = window.__zegaExplorerPushedSchema ?? '';
      const updateNote = () => {
        if (note) note.textContent = !pushedSchema ? 'schema: not pushed yet' : editor.getValue().trim() === pushedSchema.trim() ? 'schema: pushed' : 'schema: changed, not pushed';
      };
      updateNote();
      editor.onDidChangeModelContent(updateNote);
      const dialog = document.createElement('dialog');
      dialog.id = 'push-dialog';
      dialog.innerHTML = `<form method="dialog"><h2>Push schema to ${options.graphName ?? 'this graph'}?</h2><p id="push-when"></p><p id="push-error" role="alert" hidden></p><div class="remote-actions"><button id="push-cancel" type="button">Cancel</button><button id="push-confirm" class="primary" type="button">Push schema</button></div></form>`;
      root.append(dialog);
      push.onclick = async () => {
        dialog.querySelector('#push-error').hidden = true;
        dialog.querySelector('#push-when').textContent = pushedSchema ? 'This replaces the graph schema. Your data does not change. This counts as one write.' : 'The graph has no schema yet. Your data does not change. This counts as one write.';
        dialog.showModal();
      };
      dialog.querySelector('#push-cancel').onclick = () => dialog.close();
      dialog.querySelector('#push-confirm').onclick = async () => {
        const error = dialog.querySelector('#push-error');
        try {
          window.__zega.schema(editor.getValue());
          const answer = await window.__zega.pushSchema(editor.getValue());
          pushedSchema = answer.schema;
          localStorage.setItem('zega.v2.schema', pushedSchema);
          dialog.close();
          updateNote();
        } catch (reason) {
          error.textContent = reason instanceof Error ? reason.message : String(reason);
          error.hidden = false;
        }
      };
    }
  }
  if (options.chrome) {
    const conn = options.chrome.querySelector('.conn');
    if (conn && options.label) conn.querySelector('#conn-label').textContent = options.label;
  }
  return { root, toolbar: options.chrome ?? root.querySelector('#topbar'), dispose() { themeObserver.disconnect(); root.replaceChildren(); options.chrome?.replaceChildren(); styles.remove(); root.classList.remove('zega-explorer'); } };
}

export { rootSelector };
