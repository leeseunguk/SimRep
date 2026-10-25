import { surfaceKey, type RunData } from "./analysis";
import type { Trace } from "./trace";

const numberCsv = (v: number) => (Number.isFinite(v) ? String(v) : "");

/** End state of every ion (or `indices`), one row per ion. `velocity`: f64 vx, vy, vz per ion. */
export function endStatesCsv(run: RunData, velocity: Float64Array, indices?: ArrayLike<number>): string {
  const { cols, surface, surfaces } = run;
  const keys = surfaces.map((s) => surfaceKey(s.surface));
  const rows = ["ion,surface,x_mm,y_mm,z_mm,r_mm,tof_us,ke_ev,vx_mm_per_us,vy_mm_per_us,vz_mm_per_us"];
  const n = indices ? indices.length : run.n;
  for (let k = 0; k < n; k++) {
    const i = indices ? indices[k]! : k;
    rows.push(
      [
        i,
        keys[surface[i]!] ?? "",
        numberCsv(cols.x[i]!),
        numberCsv(cols.y[i]!),
        numberCsv(cols.z[i]!),
        numberCsv(cols.r[i]!),
        numberCsv(cols.tof[i]!),
        numberCsv(cols.ke[i]!),
        numberCsv(velocity[i * 3]!),
        numberCsv(velocity[i * 3 + 1]!),
        numberCsv(velocity[i * 3 + 2]!),
      ].join(","),
    );
  }
  return rows.join("\r\n") + "\r\n";
}

/** One ion's time series (f64, full precision). */
export function traceCsv(tr: Trace): string {
  const rows = ["t_us,x_mm,y_mm,z_mm,vx_mm_per_us,vy_mm_per_us,vz_mm_per_us,speed_mm_per_us,ke_ev,potential_v"];
  for (let i = 0; i < tr.t.length; i++) {
    rows.push([tr.t[i], tr.x[i], tr.y[i], tr.z[i], tr.vx[i], tr.vy[i], tr.vz[i], tr.speed[i], tr.ke[i], tr.potential[i]].join(","));
  }
  return rows.join("\r\n") + "\r\n";
}
