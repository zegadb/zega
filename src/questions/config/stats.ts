import inquirer from 'inquirer';

export async function stats() {
    const answer = await inquirer.prompt({
        type: 'list',
        name: 'result',
        message: 'Log container vitals:',
        choices: ['Never', '10 sec', '30 sec', '1 min', 'Hourly']
    });

    return answer;
}