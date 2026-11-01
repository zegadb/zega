import type { Group } from '@/mcp/groups'
import type { Tool } from '@/mcp/tool'
import { account } from '@/mcp/tools/account'
import { docs } from '@/mcp/tools/docs'
import { functions } from '@/mcp/tools/functions'
import { graphs } from '@/mcp/tools/graphs'
import { local } from '@/mcp/tools/local'
import { storage } from '@/mcp/tools/storage'

/** Every tool there is, by group. What a server offers is chosen from this (see `server.ts`). */
export const TOOLS: Record<Group, Tool[]> = { docs, account, graphs, functions, storage, local }
