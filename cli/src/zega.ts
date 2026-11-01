#!/usr/bin/env bun

import { program } from './program'

// `zega` alone shows the help (with the mark on a terminal) instead of doing nothing.
if (process.argv.length <= 2) program.help()
await program.parseAsync()
// A prompt can leave stdin open: leave on purpose, once everything is written.
process.exit(process.exitCode ?? 0)
