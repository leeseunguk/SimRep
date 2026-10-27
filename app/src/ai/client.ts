import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { AI_COMMAND, AI_EVENT, AiError, parseEvent, unwrap, type AiEvent, type Method, type Methods, type RpcResponse } from "./protocol";

let nextId = 1;
let listening: Promise<unknown> | null = null;
let handler: (event: AiEvent) => void = () => {};

/** Where `ai-event` notifications go. The listener itself is registered before the first request. */
export function onAiEvent(fn: (event: AiEvent) => void): void {
  handler = fn;
}

/**
 * One bridge request through the Tauri `ai_request` command. The `ai-event` listener is in place
 * before anything is sent (the protocol's rule), so no event of this request can be missed.
 */
export async function request<M extends Method>(method: M, params: Methods[M]["params"]): Promise<Methods[M]["result"]> {
  await (listening ??= listen(AI_EVENT, (e) => {
    const event = parseEvent(e.payload);
    if (event) handler(event);
  }));
  let response: RpcResponse<M>;
  try {
    response = await invoke<RpcResponse<M>>(AI_COMMAND, { request: { jsonrpc: "2.0", id: nextId++, method, params } });
  } catch (err) {
    // The command itself failed: no sidecar, no Node, the process went away.
    throw new AiError("BRIDGE_UNAVAILABLE", String(err));
  }
  return unwrap(response);
}
