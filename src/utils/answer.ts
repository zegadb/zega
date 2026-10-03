import { LocalFailure } from '@/utils/errors'

/**
 * One wizard question that is also a flag. The flag's value wins; otherwise a
 * terminal is asked; otherwise (an agent, CI, a pipe) the command fails naming
 * the flag. A flag's value and a typed answer pass the same `validate`, so the
 * two cannot disagree about what is acceptable.
 */
export async function answer<T>(question: {
    flag: string
    given: T | undefined
    /** Why it is needed and what it looks like, for the error without a terminal. */
    hint: string
    prompt: boolean
    validate: (value: T) => string | null
    ask: () => Promise<T>
}): Promise<T> {
    if (question.given !== undefined) {
        const bad = question.validate(question.given)
        if (bad !== null) throw new LocalFailure(`${question.flag}: ${bad}`)
        return question.given
    }
    if (!question.prompt) {
        throw new LocalFailure(
            `missing ${question.flag}: ${question.hint}. There is no terminal to ask on, so every answer must be a flag.`,
        )
    }
    return await question.ask()
}
