import type { ZodRawShape, z } from 'zod'

import type { Context, Tool } from '@/mcp/tool'
import type { Group } from '@/mcp/groups'

type Shape<S extends ZodRawShape> = { [K in keyof S]: z.infer<S[K]> }

/** A tool whose function sees its arguments typed from its input shape. */
export function tool<S extends ZodRawShape>(
    spec: Omit<Tool, 'input' | 'run' | 'manage'> & { manage?: boolean; input: S; run: (args: Shape<S>, context: Context) => Promise<string> },
): Tool {
    const { run, manage, ...rest } = spec
    return { ...rest, manage: manage ?? false, run: (args, context) => run(args as Shape<S>, context) }
}

/** Read tools carry no flags; a tool that changes something says what in its description. */
export const read = { writes: false, manage: false } as const

export type { Group }
