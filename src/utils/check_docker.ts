import http from 'http'

export async function check_docker (timeout = 2000) {
    return new Promise((resolve, reject) => {
        const socketPath = '/var/run/docker.sock'

        // Define the request options
        const requestOptions = {
            socketPath,
            path: '/_ping', // Use a Docker API endpoint for health check
            method: 'GET',
        }

        // Create an HTTP request
        const req = http.request(requestOptions)

        // Set a timeout for the request
        req.setTimeout(timeout)

        // Handle the response
        req.on('response', (res) => {
            if (res.statusCode === 200) {
                resolve(true) // Successfully connected
            } else {
                reject(new Error(`Failed to connect, status code: ${res.statusCode}`))
            }
        })

        // Handle request errors
        req.on('error', (err) => {
            reject(err)
        })

        // Send the request
        req.end()
    })
}
