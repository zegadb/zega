/**
 * Searching the documentation that ships with the CLI (`src/docs/`, zegadb/zega's `docs/*.md`).
 *
 * A page is split into sections at its `#`/`##`/`###` headings (not inside a code block); a search scores each
 * section by how many of the query's words it has, a word in a heading counting more, and answers the best ones
 * with where they are. Nothing is fetched: what is searched is what was vendored, at the commit `inventory.json` names.
 */
import { PAGES } from '@/docs/pages'

export type Section = { page: string; heading: string; text: string }

export type Hit = { page: string; heading: string; score: number; excerpt: string }

/** The page's title: its first `# ` heading, or its name. */
export function titleOf(page: string, text: string = PAGES[page] ?? ''): string {
    return /^#\s+(.+)$/m.exec(text)?.[1]?.trim() ?? page
}

export function pageNames(pages: Record<string, string> = PAGES): string[] {
    return Object.keys(pages).sort()
}

export function sectionsOf(page: string, text: string): Section[] {
    const sections: Section[] = []
    let heading = titleOf(page, text)
    let body: string[] = []
    let fence = false
    const flush = () => {
        const joined = body.join('\n').trim()
        if (joined !== '') sections.push({ page, heading, text: joined })
        body = []
    }
    for (const line of text.split('\n')) {
        if (/^```/.test(line)) fence = !fence
        const match = fence ? null : /^#{1,3}\s+(.+)$/.exec(line)
        if (match !== null) {
            flush()
            heading = match[1]!.trim()
        } else {
            body.push(line)
        }
    }
    flush()
    return sections
}

const words = (text: string): string[] => text.toLowerCase().match(/[\p{L}\p{N}_]+/gu) ?? []

/** The best sections for a query, best first. An empty query, or one no section has a word of, finds nothing. */
export function search(query: string, limit: number, pages: Record<string, string> = PAGES): Hit[] {
    const wanted = [...new Set(words(query))]
    if (wanted.length === 0) return []
    const hits: Hit[] = []
    for (const page of pageNames(pages)) {
        for (const section of sectionsOf(page, pages[page]!)) {
            const inHeading = new Set(words(`${section.heading} ${page}`))
            const inText = words(section.text)
            let score = 0
            let matched = 0
            for (const word of wanted) {
                const count = inText.filter((w) => w === word).length
                const head = inHeading.has(word) ? 1 : 0
                if (count + head > 0) matched++
                score += Math.min(count, 5) + head * 6
            }
            // Every word of the query in one section beats many hits of one word.
            if (matched > 0) hits.push({ page, heading: section.heading, score: score + matched * 4 * matched, excerpt: excerpt(section.text, wanted) })
        }
    }
    return hits.sort((a, b) => b.score - a.score || a.page.localeCompare(b.page)).slice(0, limit)
}

/** About 600 characters of the section, starting at its first line that has a word of the query. */
function excerpt(text: string, wanted: string[]): string {
    const lines = text.split('\n')
    const at = Math.max(0, lines.findIndex((line) => wanted.some((word) => line.toLowerCase().includes(word))))
    const out = lines.slice(at).join('\n')
    return out.length <= 600 ? out : out.slice(0, 600) + '...'
}
