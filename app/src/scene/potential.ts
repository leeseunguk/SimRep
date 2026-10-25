/** 0..1 amount of the positive/negative colour mixed into the neutral electrode colour. */
export function potentialAmount(voltage: number, scale: number): number {
  return Math.min(1, Math.abs(voltage) / Math.max(scale, 1e-9)) ** 0.7;
}
