import inquirer from 'inquirer';

export async function iport(internalPort: any) {
    const answer = await inquirer.prompt({
        name: 'result',
        message: 'Internal port:',
        default: `${internalPort ?? 8080}`,
    });

    return answer;
}