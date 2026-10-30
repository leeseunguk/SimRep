import { AsyncLocalStorage } from 'node:async_hooks'

/** Shared with synchronous workflow work, which child-process timers cannot interrupt. */
export const workflowDeadline = new AsyncLocalStorage()
export function withinWorkflowDeadline(timeoutSeconds, run) {
  return args => workflowDeadline.run(Date.now() + timeoutSeconds * 1000, () => run(args))
}
