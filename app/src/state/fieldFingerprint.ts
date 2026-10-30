import type { PaInfo, FastStatus } from "../scene/pa";
import { fileSha256 } from "./projectDir";
import { canonicalJson, sha256Hex } from "./runFormat";

/** Exactly the file family loaded by the desktop session (including magnetic PA maps). */
export async function fieldFingerprint(path: string, info: PaInfo, fast: FastStatus | null, surfaces: boolean): Promise<string> {
  const files: [string, string][] = [["pa", path]];
  if (fast?.state === "ready") {
    const prefix = path.match(/\.([pP][aA])0$/)?.[1];
    if (!prefix) throw new Error("fast-adjust field must have a PA0 path");
    for (const { number } of [...fast.electrodes].sort((a, b) => a.number - b.number)) {
      const suffix = number === 0 ? "_" : String(number);
      files.push([`basis:${suffix}`, path.replace(/\.[^.\\/]+$/, `.${prefix}${suffix}`)]);
    }
  }
  if (surfaces && info.cuts) files.push(["cuts", path.replace(/[^\\/]*$/, info.cuts)]);
  const hashes = await Promise.all(files.map(async ([role, file]) => [role, await fileSha256(file)]));
  return sha256Hex(canonicalJson(hashes));
}
