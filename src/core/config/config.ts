import os from 'node:os'
import path from 'node:path'
import fs from 'node:fs'

import * as utils from '../../utils'

// Use the user's home directory to store the application data
const getAppDataPath = path.join(os.homedir(), '.kebele')

const configPath = path.join(getAppDataPath, 'kb.json');

if (!fs.existsSync(getAppDataPath)) {
    fs.mkdirSync(getAppDataPath, { recursive: true });
}

// defaults
// const config = {}
// config.webhook = 'No'
// config.url = 'ghcr.io/samifouad/hello-world-docker'
// config.ePort = 80
// config.iPort = 8080
// config.addEnv = 'No'
// config.cron = 'Every minute'

export async function config() {

        await utils.welcome("config")

    //     try {
            

    //     } catch (error) {


    //     }
    // }
}