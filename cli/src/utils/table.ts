/** `-` for what the API leaves null or out. */
export const NONE = '-'

/** An aligned table: header in capitals, two spaces between columns, numbers right-aligned, no trailing spaces. */
export class Table {
    private readonly rows: string[][] = []

    constructor(private readonly columns: { header: string; right?: boolean }[]) {}

    row(cells: string[]): void {
        if (cells.length !== this.columns.length) throw new Error('internal error: a row has the wrong number of cells')
        this.rows.push(cells)
    }

    render(): string {
        const width = (column: number) =>
            Math.max(
                (this.columns[column]?.header ?? '').length,
                ...this.rows.map((row) => Array.from(row[column] ?? '').length),
            )
        const widths = this.columns.map((_, column) => width(column))
        const line = (cells: string[]) =>
            cells
                .map((cell, column) => {
                    const pad = ' '.repeat((widths[column] ?? 0) - Array.from(cell).length)
                    return this.columns[column]?.right ? pad + cell : cell + pad
                })
                .join('  ')
                .trimEnd() + '\n'
        return line(this.columns.map((c) => c.header)) + this.rows.map(line).join('')
    }
}

/** Labelled lines for one resource. */
export function detail(rows: [label: string, value: string][]): string {
    const width = Math.max(0, ...rows.map(([label]) => label.length))
    return rows.map(([label, value]) => `${label.padEnd(width)}  ${value}\n`).join('')
}
