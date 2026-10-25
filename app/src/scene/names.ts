import type { SceneObject } from "../state/store";

export function objectName(o: SceneObject): string {
  return o.customName ?? o.fileName;
}
