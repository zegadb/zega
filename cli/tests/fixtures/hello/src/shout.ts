export function shout(text: string): string {
    // A library check on NODE_ENV: the bundle must not read `process` at run time.
    return process.env.NODE_ENV === 'production' ? text.toUpperCase() : text
}
