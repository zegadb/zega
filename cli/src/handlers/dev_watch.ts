import fs from 'node:fs'

/** Directories whose changes never need a rebuild (dependencies, version control, output, scratch). */
const IGNORED = new Set(['node_modules', '.git', 'dist', '.tmp', '.bun-cache'])

/**
 * Call `onChange` (after a short quiet period, once for a burst of saves) when a file under `dir`
 * changes. Returns what stops the watching.
 */
export function watchProject(dir: string, onChange: () => void, quietMs = 60): () => void {
    let timer: ReturnType<typeof setTimeout> | null = null
    const watcher = fs.watch(dir, { recursive: true }, (_event, filename) => {
        if (filename === null) return
        if (filename.split(/[\\/]/).some((part) => IGNORED.has(part))) return
        if (timer !== null) clearTimeout(timer)
        timer = setTimeout(() => {
            timer = null
            onChange()
        }, quietMs)
    })
    return () => {
        if (timer !== null) clearTimeout(timer)
        watcher.close()
    }
}
