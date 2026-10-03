import inquirer from 'inquirer'
import chalk from 'chalk'

import * as core from '@/core'
import * as utils from '@/utils'

export async function eport(externalPort: any) {
    const answer = await inquirer.prompt({
        name: 'result',
        message: 'External port:',
        default: `${externalPort ?? 80}`,
    })

    try {

        const query = core.db.prepare("SELECT COUNT(ePort) as count from containers where ePort = ?")    

        const count: any = query.all(answer.result)

        if (count[0] && count[0].count > 0) {
            console.error('')
            console.error(chalk.red('error') +': external port already exists')
            console.error('')
            console.error('please pick a new one or report issues at https://kebele.dev')
            console.error('')
            process.exit(1)
        }
       
        await utils.sleep(300)

    } catch (error) {
        console.error('operation failed')
        console.error('')
        console.error('please try again or report issues at https://kebele.dev')
        console.error('')
        process.exit(1)
    }

    return answer
}