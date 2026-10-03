// node
import os from 'node:os'
import path from 'node:path'
import fs from 'node:fs'
import Database from 'better-sqlite3'

export { config } from './config/config'
export { add } from './add/add'
export { status } from './status/status'

// Use the user's home directory to store the application data
const appDataPath = path.join(os.homedir(), '.kebele')

const dbPath = path.join(appDataPath, 'kb.db');

if (!fs.existsSync(appDataPath)) {
    fs.mkdirSync(appDataPath, { recursive: true });
}

const db = new Database(dbPath);

export {
    appDataPath,
    dbPath,
    db
}