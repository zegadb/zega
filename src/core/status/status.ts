// @ts-nocheck

import { createSpinner } from 'nanospinner'
import chalk from 'chalk'
import * as core from '@/core'
import { docker_request } from '@/utils'

export async function status() {
    const spinner = createSpinner('task: check docker status').start();

    try {
        const response: any = await docker_request('/containers/json?all=true')

        if (!response.statusCode || response.statusCode !== 200) {
            throw ({ message: 'error: unusual response from Docker daemon'})
        }
        const containers = response.data
        const containerCount = containers.length
        if (containerCount === 0) {
            console.log('No containers found')
        }

        console.log('Containers')
        containers.forEach(container => {
            let currentVersion = container.Labels['org.opencontainers.image.version'] ?? 'unknown'
            let currentStatus = container.Status.startsWith("Up") ? chalk.green(container.Status) : chalk.red(container.Status)
            let currentPorts = []
            container.Ports.forEach(port => { if (port.IP === '0.0.0.0' || port.IP === '127.0.0.1' || port.IP === 'localhost'){ currentPorts.push(port) } })
            let portInfo = ''
            if (currentPorts.length === 0) {
                portInfo = 'none exposed'

            } else if (currentPorts.length === 1) {

                portInfo = currentPorts[0].IP +':'+ currentPorts[0].PublicPort + '->' + currentPorts[0].PrivatePort + '/' + currentPorts[0].Type + ' '
            
            } else {
                portInfo = 'Multiple'
            }
            console.log('ID: '+ chalk.blue(container.Id.substr(0, 8)) + 
                        ' - Image: '+ chalk.blue(container.Image) +  
                        ' ('+ chalk.magenta(currentVersion) +')' +
                        ' - Ports: '+ chalk.blue(portInfo) +' - Status: '+ currentStatus
                        )
            console.log('')
        })

        // Future implementations can go here for local images and cron jobs

        spinner.success()
        process.exit(0)
    } catch (error: any) {
        spinner.error()
        console.error(error.message ?? 'operation failed.')
        console.log('')
        console.log('please try again or report issues at https://kebele.dev')
        console.log('')
        process.exit(1)
    }
}