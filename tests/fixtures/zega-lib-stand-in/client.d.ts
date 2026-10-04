export function connect(options: { url: string; key: string }): {
    query<T = unknown>(zql: string): Promise<T>
    mutate<T = unknown>(zql: string): Promise<T>
}
