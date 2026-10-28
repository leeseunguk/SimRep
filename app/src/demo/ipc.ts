import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import type { Channel } from "@tauri-apps/api/core";
import fixtureText from "./fixtures/einzel.json?raw";
import type { RpcRequest } from "../ai/protocol";
import { createDemoAi, type DemoAi } from "./ai";

/**
 * Demo mode (VITE_SIMREP_DEMO=1, `npm run dev:demo`, the e2e smoke test): the UI runs as the
 * desktop app against a mocked Tauri IPC layer that answers with packets recorded from the real
 * native core (scripts/capture-demo-fixtures.mjs), so the real decoders, stores and views run
 * without a GPU or a native window. Only the einzel template exists; voltages and ion settings do
 * not change the recorded field or flights. The Rust side is not involved.
 */

interface Fixture {
  created: { path: string; name: string; size: number; dir: string };
  info: { path: string; name: string; size: number }[];
  /** base64 packets (SPA1, SPQ1, STR4, SIT1) as the native core sent them. */
  opened: string;
  targets: unknown;
  messages: ({ at: number; json: Record<string, unknown> } | { at: number; packet: string })[];
  labels: string;
  fast: string;
  flights: Record<"live" | "precise", { packet: string; velocities: string }>;
  trace: string;
  computeDevice: unknown;
}

/** What the demo saw, for the e2e test (window.__simrepDemo). */
export interface DemoLog {
  calls: string[];
  /** write_bytes bodies, with the path the mocked save dialog answered. */
  writes: { path: string | null; text: string }[];
  /** Refine messages sent per job, in order: the message type, or the magic of a binary packet. */
  refine: Record<number, string[]>;
  /** Resolves with the latest answer once `cmd` has been answered `count` times (at once if it already was). */
  answered(cmd: string, count?: number): Promise<unknown>;
  /** With `?replay=manual` (e2e): send the next message of refine `job`; its type, or null once the job has ended. */
  step(job: number): string | null;
  /** Files the demo keeps (project files, X.simrep.d), path -> size; they survive a reload. */
  files(): Record<string, number>;
  /** Text of a kept file (null when missing). */
  text(path: string): string | null;
  /** Changes the sha256 the demo reports for `path` (an array edited on disk). */
  setSha(path: string, sha: string): void;
}

/**
 * Demo files: project files and their data folders, in localStorage (base64) so a reload, like
 * reopening the app, still finds them. Data folder paths follow src-tauri/src/projectdir.rs.
 */
const FS_KEY = "simrep.demo.fs";
const SHA_KEY = "simrep.demo.sha";
const toB64 = (b: Uint8Array) => {
  let s = "";
  for (let i = 0; i < b.length; i += 0x8000) s += String.fromCharCode(...b.subarray(i, i + 0x8000));
  return btoa(s);
};
const store = <T,>(key: string): Record<string, T> => JSON.parse(localStorage.getItem(key) ?? "{}") as Record<string, T>;
const keep = (key: string, v: unknown) => localStorage.setItem(key, JSON.stringify(v));

function relOk(rel: string, allowEmpty = false): boolean {
  if (rel === "") return allowEmpty;
  return !/[\\:]/.test(rel) && !rel.startsWith("/") && rel.replace(/\/$/, "").split("/").every((s) => s !== "" && s !== "." && s !== "..");
}
function dataPath(project: string, rel: string, allowEmpty = false): string {
  if (!/\.simrep$/i.test(project) || !relOk(rel, allowEmpty)) throw { code: "unexpected", detail: `path leaves the project folder: ${rel}` };
  const sep = project.includes("\\") ? "\\" : "/";
  return `${project}.d${rel ? sep + rel.replace(/\/$/, "").split("/").join(sep) : ""}`;
}

/** A refine job being replayed: its channel, the index of its next message and what is left to send. */
interface Job {
  channel: Channel<unknown>;
  next: number;
  queue: unknown[];
}

/** Spacing of replayed refine messages: slow enough to watch the convergence chart grow. */
const REFINE_STEP_MS = 60;
const DOCUMENTS = "C:\\Users\\demo\\Documents";

const decode = (b64: string): ArrayBuffer => Uint8Array.from(atob(b64), (c) => c.charCodeAt(0)).buffer;

function headerOf(packet: ArrayBuffer): { token?: number } {
  const len = new DataView(packet).getUint32(4, true);
  return JSON.parse(new TextDecoder().decode(new Uint8Array(packet, 8, len))) as { token?: number };
}

/** A command error the UI states in its language (`reason.<code>`), like the native core's PaError. */
const refused = (detail: string) => ({ code: "unexpected", detail });

export function installDemo(): void {
  const fx = JSON.parse(fixtureText) as Fixture;
  // Manual replay: refine messages wait for `step` instead of a timer, so a test sees every state.
  const manual = new URLSearchParams(location.search).get("replay") === "manual";
  const live = new Map<number, Job>();
  const answers = new Map<string, { count: number; last: unknown }>();
  const waiters: { cmd: string; count: number; resolve(last: unknown): void }[] = [];

  /** Sends the next message of `job` on its channel; the job ends with its last message. */
  const send = (job: number): string | null => {
    const j = live.get(job);
    if (!j) return null;
    const message = j.queue.shift();
    const run = (window as unknown as { __TAURI_INTERNALS__: { runCallback(id: number, data: unknown): void } }).__TAURI_INTERNALS__;
    run.runCallback(j.channel.id, { index: j.next++, message });
    const type = message instanceof ArrayBuffer ? String.fromCharCode(...new Uint8Array(message, 0, 4)) : (message as { type: string }).type;
    demo.refine[job]!.push(type);
    if (j.queue.length === 0) live.delete(job);
    else if (!manual) setTimeout(() => send(job), REFINE_STEP_MS);
    return type;
  };

  const demo: DemoLog = {
    calls: [],
    writes: [],
    refine: {},
    answered: (cmd, count = 1) =>
      new Promise((resolve) => {
        const a = answers.get(cmd);
        if (a && a.count >= count) resolve(a.last);
        else waiters.push({ cmd, count, resolve });
      }),
    step: (job) => (manual ? send(job) : null),
    files: () => Object.fromEntries(Object.entries(store<string>(FS_KEY)).map(([k, v]) => [k, atob(v).length])),
    text: (path) => {
      const v = store<string>(FS_KEY)[path];
      return v === undefined ? null : new TextDecoder().decode(Uint8Array.from(atob(v), (c) => c.charCodeAt(0)));
    },
    setSha: (path, sha) => keep(SHA_KEY, { ...store<string>(SHA_KEY), [path]: sha }),
  };
  const fsGet = (path: string): Uint8Array<ArrayBuffer> | null => {
    const v = store<string>(FS_KEY)[path];
    return v === undefined ? null : Uint8Array.from(atob(v), (c) => c.charCodeAt(0));
  };
  const fsPut = (path: string, bytes: Uint8Array) => keep(FS_KEY, { ...store<string>(FS_KEY), [path]: toB64(bytes) });
  const notFound = (path: string) => ({ code: "notFound", detail: `demo mode has no ${path}` });
  (window as unknown as { __simrepDemo: DemoLog }).__simrepDemo = demo;
  // Both AI providers, mocked to the bridge protocol (src/demo/ai.ts).
  const ai = createDemoAi(manual, new URLSearchParams(location.search));
  (window as unknown as { __simrepDemoAi: DemoAi }).__simrepDemoAi = ai.demo;
  // isTauri() reads this flag; the mocks fill window.__TAURI_INTERNALS__.
  (globalThis as unknown as { isTauri: boolean }).isTauri = true;
  mockWindows("main");
  let lastSave: string | null = null;
  let jobs = 0;

  const answer = (cmd: string, last: unknown) => {
    const a = { count: (answers.get(cmd)?.count ?? 0) + 1, last };
    answers.set(cmd, a);
    for (const w of waiters.filter((w) => w.cmd === cmd && w.count <= a.count)) {
      waiters.splice(waiters.indexOf(w), 1);
      w.resolve(last);
    }
  };

  /** Headers of the call being handled (raw-body commands carry their paths there). */
  let headers: Record<string, string> = {};
  mockIPC(
    async (cmd, args) => {
      demo.calls.push(cmd);
      let last: unknown = null;
      try {
        return (last = await handle(cmd, args));
      } finally {
        answer(cmd, last);
      }
    },
    { shouldMockEvents: true },
  );
  const internals = (window as unknown as { __TAURI_INTERNALS__: { invoke(cmd: string, args: unknown, options?: { headers?: Record<string, string> }): Promise<unknown> } }).__TAURI_INTERNALS__;
  const mocked = internals.invoke;
  internals.invoke = (cmd, args, options) => {
    headers = options?.headers ?? {};
    return mocked(cmd, args, options);
  };

  async function handle(cmd: string, args: unknown): Promise<unknown> {
    const a = (args ?? {}) as Record<string, unknown>;
    switch (cmd) {
      case "ai_request":
        return ai.handle(a.request as RpcRequest);
      case "backend_info":
        return { appName: "simrep-app", appVersion: "0.1.0 (demo)", coreCrate: "simrep-core" };
      case "compute_device":
        return fx.computeDevice;
      case "create_template":
        if (a.kind !== "einzel") throw refused(`demo mode has only the einzel template, not ${String(a.kind)}`);
        return fx.created;
      case "path_info":
        return (a.paths as string[]).map((p) => fx.info.find((i) => i.path === p) ?? { path: p, name: p.split(/[\\/]/).pop(), size: 0 });
      case "open_pa": {
        if (a.path === fx.created.path) return decode(fx.opened);
        // The refined .pa0 (a saved project points at it): the refine result packet.
        const refined = fx.messages.filter((m): m is { at: number; packet: string } => "packet" in m).map((m) => decode(m.packet)).filter((b) => String.fromCharCode(...new Uint8Array(b, 0, 4)) === "SPA1").at(-1);
        if (refined && a.path === `${fx.created.dir}\\einzel.pa0`) return refined;
        throw refused(`demo mode cannot open ${String(a.path)}`);
      }
      case "close_session":
      case "remove_file":
        return null;
      case "write_text":
        fsPut(a.path as string, new TextEncoder().encode(a.contents as string));
        return null;
      case "project_dir_write": {
        const body = args instanceof Uint8Array ? args : new TextEncoder().encode(String(args));
        fsPut(dataPath(decodeURIComponent(headers["project-path"] ?? ""), decodeURIComponent(headers["rel-path"] ?? "")), body);
        return null;
      }
      case "project_dir_read": {
        const path = dataPath(a.projectPath as string, a.relPath as string);
        const bytes = fsGet(path);
        if (!bytes) throw notFound(path);
        return bytes.buffer;
      }
      case "project_dir_list": {
        const root = dataPath(a.projectPath as string, "", true);
        const dir = dataPath(a.projectPath as string, a.prefix as string, true);
        const sep = root.includes("\\") ? "\\" : "/";
        return Object.entries(store<string>(FS_KEY))
          .filter(([k]) => k.startsWith(dir + sep))
          .map(([k, v]) => ({ path: k.slice(root.length + 1).split(sep).join("/"), size: atob(v).length }))
          .sort((x, y) => x.path.localeCompare(y.path));
      }
      case "project_dir_delete": {
        const path = dataPath(a.projectPath as string, a.relPath as string);
        const sep = path.includes("\\") ? "\\" : "/";
        keep(FS_KEY, Object.fromEntries(Object.entries(store<string>(FS_KEY)).filter(([k]) => k !== path && !k.startsWith(path + sep))));
        return null;
      }
      case "file_sha256":
        return store<string>(SHA_KEY)[a.path as string] ?? "d3m0".padEnd(64, "0");
      case "electrode_labels":
        return decode(fx.labels);
      case "refine_targets":
        return fx.targets;
      case "refine_start": {
        const job = ++jobs;
        live.set(job, { channel: a.channel as Channel<unknown>, next: 0, queue: fx.messages.map((m) => ("json" in m ? m.json : decode(m.packet))) });
        demo.refine[job] = [];
        if (!manual) setTimeout(() => send(job), REFINE_STEP_MS);
        return job;
      }
      case "refine_cancel": {
        // Like the native core: a job still running ends with exactly one `cancelled` as its next
        // message instead of the rest of its replay; a job that has ended is left alone.
        const j = live.get(a.job as number);
        if (j) j.queue = [{ type: "cancelled" }];
        return null;
      }
      case "fast_adjust":
        return decode(fx.fast);
      case "fly":
        return decode(fx.flights[(a.request as { precision: "live" | "precise" }).precision].packet);
      case "flight_velocities": {
        const flight = Object.values(fx.flights).find((f) => headerOf(decode(f.packet)).token === a.token);
        if (!flight) throw { code: "export", detail: "flightExpired", reason: { code: "flightExpired" } };
        return decode(flight.velocities);
      }
      case "ion_trace":
        return decode(fx.trace);
      case "file_sha256":
        // Run history (state/runs.ts) hashes the array; the demo has one fixed array.
        return "0".repeat(64);
      case "write_bytes":
        demo.writes.push({ path: lastSave, text: new TextDecoder().decode(args as Uint8Array) });
        return null;
      case "read_text": {
        const bytes = fsGet(a.path as string);
        if (!bytes) throw notFound(String(a.path));
        return new TextDecoder().decode(bytes);
      }
      case "recovery_path":
        return "C:\\Users\\demo\\AppData\\Local\\dev.simrep.app\\recovery.simrep";
      case "plugin:dialog|save": {
        const options = (a.options ?? {}) as { defaultPath?: string };
        lastSave = options.defaultPath ?? `${DOCUMENTS}\\export`;
        return lastSave;
      }
      case "plugin:dialog|open":
        return null;
      case "plugin:dialog|ask":
      case "plugin:dialog|confirm":
      case "plugin:dialog|message":
        return true;
      case "plugin:path|resolve_directory":
        return DOCUMENTS;
      case "plugin:path|join":
        return (a.paths as string[]).join("\\");
      default:
        // Window chrome (title, size) and other plugin calls do nothing in the demo.
        if (cmd.startsWith("plugin:")) return null;
        throw refused(`demo mode has no ${cmd} command`);
    }
  }
}
