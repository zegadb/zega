import inquirer from 'inquirer';

export async function webhook_url() {
    const answer = await inquirer.prompt({
        name: 'result',
        message: 'Webhook URL:',
        default: 'https://my-cool-app.com/post/webhook',
    });

    return answer;
}