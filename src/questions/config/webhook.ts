import inquirer from 'inquirer'

export async function webhook() {
    const answer = await inquirer.prompt({
        type: 'list',
        name: 'result',
        message: 'Send webhook on container update:',
        choices: ['No', 'Yes']
    });

    return answer;
}