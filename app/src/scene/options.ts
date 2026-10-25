/**
 * Every value the UI lets the user pick that a native-core command must accept. The controls and
 * the request types in backend.ts are built from these lists; options.test.ts checks them against
 * engineOptions.json, which the Rust side generates from what its commands parse
 * (src-tauri/src/options.rs), so neither side can add or rename one silently.
 */

export const REFINE_DEVICES = ["auto", "gpu", "cpu"] as const;
export const FLY_PRECISIONS = ["live", "precise"] as const;
export const FLY_COLLISIONS = ["allCorners", "nearestNode"] as const;
export const BEAM_SHAPES = ["point", "line", "disk"] as const;
/** SIMION array exports, in the order the export dialog offers them. */
export const PA_EXPORT_FORMATS = ["pa0", "pa", "paHash"] as const;
export const TEMPLATE_IDS = ["einzel", "quadrupole", "reflectron", "trap"] as const;
export const BUILD_METHODS = ["cutcell", "aligned"] as const;
export const CAD_SURFACES = ["tessellated", "exact"] as const;
