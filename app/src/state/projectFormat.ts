import { DEFAULT_KEEP_RUNS, DEFAULT_RUN_CAP_MB } from "./runFormat";

/**
 * Version handling of `.simrep` files (state/project.ts has the full document type). Format 1:
 * arrays, meshes, view and a summary of the last flight. Format 2 adds `runs` (history settings)
 * and per array `run`, the stored run in `X.simrep.d/runs/<id>/` it showed (docs/project-format.md).
 */

export const PROJECT_FORMAT = "simrep-project";
export const PROJECT_VERSION = 2;

interface LooseDoc {
  format?: unknown;
  version?: unknown;
  arrays?: ({ run?: unknown } & Record<string, unknown>)[];
  meshes?: unknown[];
  runs?: { keep?: number; capMb?: number };
}

/** Any supported project file, migrated to the current format (format 1: no runs, every array flies again). */
export function parseProject(text: string) {
  const doc = JSON.parse(text) as LooseDoc;
  if (doc.format !== PROJECT_FORMAT || typeof doc.version !== "number") throw new Error("not a SimRep project");
  const version = doc.version;
  if (version > PROJECT_VERSION) throw new Error(`made by a newer SimRep (format ${version})`);
  const runs = { keep: DEFAULT_KEEP_RUNS, capMb: DEFAULT_RUN_CAP_MB, ...(version >= 2 ? doc.runs : null) };
  return {
    ...doc,
    version: PROJECT_VERSION,
    arrays: (doc.arrays ?? []).map((a) => ({ ...a, run: version >= 2 && typeof a.run === "string" ? a.run : null })),
    meshes: doc.meshes ?? [],
    runs,
  };
}
