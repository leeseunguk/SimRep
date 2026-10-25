import { describe, expect, it } from "vitest";
import engine from "./engineOptions.json";
import { BEAM_SHAPES, BUILD_METHODS, CAD_SURFACES, FLY_COLLISIONS, FLY_PRECISIONS, PA_EXPORT_FORMATS, REFINE_DEVICES, TEMPLATE_IDS } from "./options";
import { TEMPLATES } from "./templates";

// engineOptions.json is what the native core's commands accept (src-tauri/src/options.rs, whose
// tests keep it current). The UI offers exactly those values, so neither side drifts silently.
const same = (ui: readonly string[], native: string[]) => expect([...ui].sort()).toEqual([...native].sort());

describe("UI options match the native core", () => {
  it("refine devices", () => same(REFINE_DEVICES, engine.refineDevices));
  it("flight precisions", () => same(FLY_PRECISIONS, engine.flyPrecisions));
  it("electrode collision rules", () => same(FLY_COLLISIONS, engine.collisions));
  it("beam shapes", () => same(BEAM_SHAPES, engine.beamShapes));
  it("SIMION array export formats", () => same(PA_EXPORT_FORMATS, engine.paExportFormats));
  it("templates", () => {
    same(TEMPLATE_IDS, engine.templates);
    same(TEMPLATES.map((t) => t.id), engine.templates);
  });
  it("geometry build methods", () => same(BUILD_METHODS, engine.buildMethods));
  it("CAD surfaces", () => same(CAD_SURFACES, engine.cadSurfaces));
  it("covers every list the native core exports", () => {
    expect(Object.keys(engine).sort()).toEqual(["beamShapes", "buildMethods", "cadSurfaces", "collisions", "flyPrecisions", "paExportFormats", "refineDevices", "templates"]);
  });
});
