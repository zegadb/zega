import { createSpinner } from 'nanospinner'
import * as utils from '@/utils'

export async function cron(config: any) {
    const spinner = createSpinner('running...').start()

    await utils.sleep(500)

    spinner.success({ text: 'Ok'})

    return '1'
}