/**
 * Timing of the results plots, for the panel header and for measuring "plots follow a live re-fly"
 * (docs/dev/frontend.md): analysis time of the last update and when each chart was last painted.
 */
export const plotPerf = {
  /** ms spent binning / filtering for the last flight shown. */
  analysisMs: 0,
  /** performance.now() of the last chart paint. */
  lastDraw: 0,
  draws: 0,
  drawn() {
    this.lastDraw = performance.now();
    this.draws++;
  },
};

/** Runs `fn` and records its duration as the analysis time. */
export function timed<T>(fn: () => T): T {
  const t0 = performance.now();
  const out = fn();
  plotPerf.analysisMs = performance.now() - t0;
  return out;
}
