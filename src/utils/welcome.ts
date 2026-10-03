import chalk from 'chalk'
import pkg from '../../package.json'

export async function welcome(cmd: any) {
    
    const pkgVer = pkg.version

    let greeting

    switch (cmd) {
        case 'add':
            greeting = 'setup a new container'
        break;

        case 'config':
            greeting = 'configure runtime settings'
        break;

        case 'status':
            greeting = 'list all docker containers'
        break;

        default:
            greeting = 'welcome!'
        break;
    }

    console.log(chalk.bgBlue(' kebele ') +' ' + chalk.blue(pkgVer) + ' | ' + chalk.green(cmd) + ' - '+ greeting +' \n')
}