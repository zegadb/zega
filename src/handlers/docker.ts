import { createSpinner } from 'nanospinner'
import chalk from 'chalk'

import * as core from '@/core'
import * as utils from '@/utils'

export async function docker(config: any) {
    const spinner = createSpinner('running...').start()

    try {

        const query = core.db.prepare("SELECT name, url, ePort, iPort from containers where id = ?")    

        const container = query.all(config.id)

        console.log(container)

        if (!container[0]) {
            console.error('')
            console.error(chalk.red('error') +': container name already exists')
            console.error('')
            console.error('please pick a new name or report issues at https://kebele.dev')
            console.error('')
            process.exit(1)
        }
       
        await utils.sleep(300)

    } catch (error) {
        console.error('operation failed')
        console.error(error)
        console.error('')
        console.error('please try again or report issues at https://kebele.dev')
        console.error('')
        process.exit(1)
    }

    spinner.success({ text: 'Ok'})
}