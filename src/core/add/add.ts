// @ts-nocheck
import chalk from 'chalk'
import { customAlphabet } from 'nanoid'
import { createSpinner } from 'nanospinner'

// core/helper functions
import { db } from '../index.ts'

// setup questions
import * as questions from '@/questions'

// handle answers
import * as handlers from '@/handlers'

if (db) {
    db.transaction(() => {
        db.prepare(`CREATE TABLE IF NOT EXISTS containers (
            id TEXT PRIMARY KEY,
            json_config TEXT,
            name TEXT as (json_extract(json_config, '$.name')) STORED UNIQUE,
            url TEXT as (json_extract(json_config, '$.url')) STORED UNIQUE,
            ePort TEXT as (json_extract(json_config, '$.ePort')) STORED UNIQUE,
            iPort TEXT as (json_extract(json_config, '$.iPort')) STORED,
            json_runtime TEXT,
            cid TEXT as (json_extract(json_runtime, '$.cid')) STORED UNIQUE,
            version TEXT as (json_extract(json_runtime, '$.version')) STORED,
            protocol TEXT as (json_extract(json_runtime, '$.protocol')) STORED,
            status TEXT as (json_extract(json_runtime, '$.status')) STORED,
            public TEXT as (json_extract(json_runtime, '$.public')) STORED,
            last_updated DATETIME DEFAULT CURRENT_TIMESTAMP
        )`).run()

        // create indexes
        db.prepare(`CREATE INDEX IF NOT EXISTS idx_cid ON containers(cid)`).run()
        db.prepare(`CREATE INDEX IF NOT EXISTS idx_url ON containers(url)`).run()
        db.prepare(`CREATE INDEX IF NOT EXISTS idx_name ON containers(name)`).run()
    })()
}

// generate a random id
const nanoid = customAlphabet('123456789ABCDEFGHIJKLMNPRSTWXYZabcdefhijklmnprstwxyz', 8)

// defaults
const config = {}
config.id = nanoid() // has index
config.name = '' // has unique constraint, has index
config.url = '' // has unique constraint, has index
config.ePort = 80 // has unique constraint
config.iPort = 8080
config.cron = 'Every minute'
config.update = 'Stop Container, Apply Update & Restart Container'

export async function add() {
    // welcome message
    await core.welcome("add")

    // check if docker available or exit
    try {

        await core.checkDocker()

    } catch (err) {
        console.error(`could not connect to 2375 on localhost`)
        console.error('')
        console.error('please make sure docker daemon is running & try again')
        console.error('')
        console.error('learn how to enable it & report issues at https://kebele.dev')
        console.error('')
        process.exit(1)
    }

    // name of container
    await questions.name().then(async (answer) => {
        const spinner_questions = createSpinner('Loading...').start()
        config.name = answer.result
        await core.sleep(500)
        spinner_questions.success({ text: 'Ok'})
    })
    
    // url of container
    await questions.url().then(async (answer) => {
        const spinner_questions = createSpinner('Loading...').start()
        config.url = answer.result
        await core.sleep(500)
        spinner_questions.success({ text: 'Ok'})
    })

    // external port
    await questions.ePort().then(async (answer) => {
        const spinner_ePort = createSpinner('Loading...').start()
        config.ePort = answer.result
        await core.sleep(500)
        spinner_ePort.success({ text: 'Ok'})
    })

    // internal port
    await questions.iPort().then(async (answer) => {
        const spinner_iPort = createSpinner('Loading...').start()
        config.iPort = answer.result
        await core.sleep(500)
        spinner_iPort.success({ text: 'Ok'})
    })

    // cron job for container updates
    await questions.cron().then(async (answer) => {
        const spinner_cron = createSpinner('Loading...').start()
        config.cron = answer.result
        await core.sleep(500)
        spinner_cron.success({ text: 'Ok'})
    });

    // how to handle container updates
    await questions.update().then(async (answer) => {
        const spinner_update = createSpinner('Loading...').start()
        config.update = answer.result
        await core.sleep(500)
        spinner_update.success({ text: 'Ok'})
    });

    //
    // HANDLE ANSWERS
    //

    console.log('\n💾 Writing to local db')
    await handlers.db(db, config, "insert_container").then(async (err) => {
        // good
    }).catch((err) => {
        console.log(err.message ?? 'Operation failed. Please try again or report issues at https://kebele.dev');
        process.exit(1)
    })
    console.log('\n🐋 Adding container to Docker ')
    await handlers.docker(config).then(async (err) => {
        // good
    }).catch((err) => {
        console.log(err.message ?? 'Operation failed. Please try again or report issues at https://kebele.dev');
        process.exit(1)
    })

    console.log('\n⏰ Creating cron job for container updates')
    await handlers.cron(config).then(async (err) => {
        // good
    }).catch((err) => {
        console.log(err.message ?? 'Operation failed. Please try again or report issues at https://kebele.dev');
        process.exit(err.code ?? 1)
    })

    console.log('\n✅ Testing everything')
    await core.runner()
    console.log('\n🥳 You\'re good!')

    console.log('\nUse ' + chalk.blue('kebele --help') +' for guidance & report issues at https://kebele.dev')

    console.log('\nRun ' + chalk.blue('kebele status') +' to check on your containers\n')
}

// run a handler depending on user selection
//
// unused now but will be used later
//
// if (config.git === 'Yes') {
//     console.log('\n Setting up git repo')
//     await handlers.git()
// }