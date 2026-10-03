#!/usr/bin/env node

import { Command } from 'commander'
import * as core from './core/index'
import pkg from '../package.json'

const program = new Command()

program
    .name('kebele')
    .version(pkg.version)
    .description('https://kebele.dev')

program
    .command('add')
    .description('setup a new container')
    .action(async () => {
        await core.add()
    })

program
    .command('config')
    .description('configure runtime settings')
    .action(async () => {
        await core.config()
    })

program
    .command('status')
    .description('list all docker containers')
    .action(async () => {
        await core.status()
    })

program.parse()

// try {
//     yargs(hideBin(process.argv))
//     .scriptName("kebele")
//     .command(commands.config)
//     .command(commands.add)
//     .command(commands.status)
//     .demandCommand(0)
//     .help()
//     .parse()
    
// } catch (err) {
//     console.error(err.message ?? 'operation failed')
//     console.log('')
//     console.log('please try again or report issues at https://kebele.dev')
//     console.log('')
//     process.exit(1)
// }