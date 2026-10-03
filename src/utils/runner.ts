import { createSpinner } from 'nanospinner';

const sleep = (ms = 2000) => new Promise((resolve) => setTimeout(resolve, ms));

export default async function handler() {
    const spinner = createSpinner('Loading...').start();
    await sleep(300);
    spinner.success({ text: 'Ok'});
}