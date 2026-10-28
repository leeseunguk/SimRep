import { invoke, isTauri } from "@tauri-apps/api/core";
import { useApp } from "./store";

/**
 * The project data folder `X.simrep.d/` next to the saved project `X.simrep` (src-tauri/src/
 * projectdir.rs): stored flight runs (`runs/<id>/`, state/runs.ts) and other per-project data.
 *
 * Paths are relative to that folder and `/` separated (`runs/<id>/run.json`); the native side
 * refuses absolute paths, `..` and symbolic links. Untitled projects have no folder yet: the
 * functions then do nothing and answer null / false / [] (also in the browser preview).
 */

export interface ProjectFileEntry {
  /** Relative to the data folder, '/' separated. */
  path: string;
  size: number;
}

/** The saved project file the folder belongs to (null while untitled or in the browser). */
export function projectFilePath(): string | null {
  return isTauri() ? useApp.getState().project.path : null;
}

/** `X.simrep.d` of the open project, or null while it is untitled. */
export function projectDataDir(project = projectFilePath()): string | null {
  return project ? `${project}.d` : null;
}

/** Writes text (UTF-8) or bytes to `rel`; false when there is no project folder. */
export async function writeProjectFile(rel: string, data: string | Uint8Array<ArrayBuffer>, project = projectFilePath()): Promise<boolean> {
  if (!project) return false;
  const bytes = typeof data === "string" ? new TextEncoder().encode(data) : data;
  await invoke("project_dir_write", bytes, { headers: { "project-path": encodeURIComponent(project), "rel-path": encodeURIComponent(rel) } });
  return true;
}

/** Bytes of `rel`, or null when the file (or the project folder) does not exist. */
export async function readProjectFile(rel: string, project = projectFilePath()): Promise<Uint8Array<ArrayBuffer> | null> {
  if (!project) return null;
  try {
    return new Uint8Array(await invoke<ArrayBuffer>("project_dir_read", { projectPath: project, relPath: rel }));
  } catch (err) {
    if ((err as { code?: string } | null)?.code === "notFound") return null;
    throw err;
  }
}

/** `rel` as UTF-8 text, or null when missing. */
export async function readProjectText(rel: string, project = projectFilePath()): Promise<string | null> {
  const bytes = await readProjectFile(rel, project);
  return bytes && new TextDecoder().decode(bytes);
}

/** Every file under folder `prefix` ("" = the whole data folder), recursively, sorted by path. */
export async function listProjectFiles(prefix = "", project = projectFilePath()): Promise<ProjectFileEntry[]> {
  if (!project) return [];
  return invoke<ProjectFileEntry[]>("project_dir_list", { projectPath: project, prefix });
}

/** Deletes a file or a folder (recursively); false when there is no project folder. */
export async function deleteProjectFile(rel: string, project = projectFilePath()): Promise<boolean> {
  if (!project) return false;
  await invoke("project_dir_delete", { projectPath: project, relPath: rel });
  return true;
}

/** Lowercase hex SHA-256 of any file on disk (a potential array), via the native core. */
export function fileSha256(path: string): Promise<string> {
  return invoke<string>("file_sha256", { path });
}
