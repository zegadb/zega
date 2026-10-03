// @ts-nocheck
import inquirer from 'inquirer'
import chalk from 'chalk'
import { customAlphabet } from 'nanoid'

import * as core from '@/core'


// generate a random id
const nanoid = customAlphabet('0123456789', 5)

export async function name() {
    const answer = await inquirer.prompt({
        name: 'result',
        message: 'Container name:',
        default: 'container'+ nanoid(),
    });

    try {

        const query = core.db.prepare("SELECT COUNT(name) as count from containers where name = ?")    

        const count = query.all(answer.result)

        if (count[0] && count[0].count > 0) {
            console.error('')
            console.error(chalk.red('error') +': container name already exists')
            console.error('')
            console.error('please pick a new name or report issues at https://kebele.dev')
            console.error('')
            process.exit(1)
        }
       
        await core.sleep(300)

    } catch (error) {
        console.error('operation failed')
        console.error('')
        console.error('please try again or report issues at https://kebele.dev')
        console.error('')
        process.exit(1)
    }

    return answer;
}