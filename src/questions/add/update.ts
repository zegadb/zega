import inquirer from 'inquirer';

export async function update() {
    const answer = await inquirer.prompt({
        type: 'list',
        name: 'result',
        message: 'Update strategy:',
        choices: ['Stop Container, Apply Update & Restart Container', 'Increment Port, Send Webhook & Schedule Deletion']
    });

    return answer;
}