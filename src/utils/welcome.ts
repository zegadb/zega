import chalk from 'chalk'
import pkg from '../../package.json'
import { write } from '@/utils/output'

/** The banner a wizard prints when it is about to ask questions on a terminal. */
export async function welcome(command: string, about: string): Promise<void> {
    await write('stdout', `${chalk.bgYellow.black(' zega ')} ${chalk.yellow(pkg.version)} | ${chalk.green(command)} - ${about}\n\n`)
}
