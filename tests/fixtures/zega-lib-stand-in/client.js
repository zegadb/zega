// POST {url}/zql with the graph key as a bearer token, as zega-server and Zega Cloud graphs take it.
export function connect({ url, key }) {
    const send = async (query) => {
        const response = await fetch(`${url}/zql`, {
            method: 'POST',
            headers: { authorization: `Bearer ${key}`, 'content-type': 'application/json' },
            body: JSON.stringify({ query }),
        })
        return (await response.json()).result
    }
    return { query: send, mutate: send }
}
