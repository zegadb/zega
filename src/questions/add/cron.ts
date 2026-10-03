import inquirer from 'inquirer'

export async function cron() {
    const answer = await inquirer.prompt({
        type: 'list',
        name: 'result',
        message: 'Check for new container version:',
        choices: ['Every minute', '5 min', '15 min', '30 min', 'Hourly', 'Weekly', 'Daily', 'Never']
    });

    return answer;
}