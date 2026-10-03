import inquirer from 'inquirer';

export async function log() {
    const answer = await inquirer.prompt({
        type: 'list',
        name: 'result',
        message: 'Capture container logs:',
        choices: ['No', 'Yes']
    });

    return answer;
}