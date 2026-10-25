import type { MessageKey } from "../i18n/en";

export type FormatId = "stl" | "gem" | "pa" | "paHash" | "pa0" | "ion" | "fly2" | "iob" | "obj" | "ply" | "3mf" | "gltf" | "step" | "iges";

export interface FileFormat {
  id: FormatId;
  extensions: readonly string[];
  labelKey: MessageKey;
  /** false = recognized and listed in the scene, import lands in a later milestone. */
  importable: boolean;
  /** Geometry that becomes electrodes through the import wizard (desktop). */
  wizard?: boolean;
}

export const FORMATS: readonly FileFormat[] = [
  { id: "stl", extensions: [".stl"], labelKey: "format.stl", importable: true, wizard: true },
  { id: "gem", extensions: [".gem"], labelKey: "format.gem", importable: true, wizard: true },
  { id: "obj", extensions: [".obj"], labelKey: "format.obj", importable: true, wizard: true },
  { id: "ply", extensions: [".ply"], labelKey: "format.ply", importable: true, wizard: true },
  { id: "3mf", extensions: [".3mf"], labelKey: "format.3mf", importable: true, wizard: true },
  { id: "gltf", extensions: [".gltf", ".glb"], labelKey: "format.gltf", importable: true, wizard: true },
  { id: "iob", extensions: [".iob"], labelKey: "format.iob", importable: true },
  { id: "pa", extensions: [".pa"], labelKey: "format.pa", importable: true },
  { id: "paHash", extensions: [".pa#"], labelKey: "format.paHash", importable: true },
  { id: "pa0", extensions: [".pa0"], labelKey: "format.pa0", importable: true },
  { id: "ion", extensions: [".ion"], labelKey: "format.ion", importable: true },
  { id: "fly2", extensions: [".fly2"], labelKey: "format.fly2", importable: true },
  { id: "step", extensions: [".step", ".stp"], labelKey: "format.step", importable: true, wizard: true },
  { id: "iges", extensions: [".iges", ".igs"], labelKey: "format.iges", importable: true, wizard: true },
];

/** Extensions that open right away (for hints such as the drop overlay). */
export const READY_EXTENSIONS = FORMATS.filter((f) => f.importable).flatMap((f) => f.extensions);

/** One representative extension per not-yet-importable format (for hints such as the drop overlay). */
export const COMING_SOON_EXTENSIONS = FORMATS.filter((f) => !f.importable).map((f) => f.extensions[0]!);

export const ACCEPT_ATTRIBUTE = FORMATS.flatMap((f) => f.extensions).join(",");

export function detectFormat(fileName: string): FileFormat | undefined {
  const name = fileName.toLowerCase();
  return FORMATS.find((f) => f.extensions.some((ext) => name.endsWith(ext)));
}

export function formatById(id: FormatId): FileFormat {
  return FORMATS.find((f) => f.id === id)!;
}
