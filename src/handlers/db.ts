import { createSpinner } from 'nanospinner'
import * as core from '@/core'
import * as utils from '@/utils'

const sleep = (ms = 2000) => new Promise((resolve) => setTimeout(resolve, ms))

export async function db(db: any, config: any, action: any) {
    const spinner = createSpinner('running...').start()

    try {
        switch (action) {
            case 'insert_container':
            
                const query = core.db.prepare("INSERT INTO containers (id, json_config) VALUES (?, json(?))")

                const result = query.run(config.id, JSON.stringify(config))
                
                await utils.sleep(300)
                
            break;
            default:
                await utils.sleep(500)
            break;
        }
    } catch (error) {
        console.error('operation failed:')
        console.error(error)
        console.error('')
        console.error('please try again or report issues at https://kebele.dev')
        console.error('')
        process.exit(1)
    }

    spinner.success({ text: 'Ok'})
}