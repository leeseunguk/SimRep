// Run in the release app loading NODE_ENV=production Vite, at 1440 x 900.
const { useApp } = await import("/src/state/store.ts");
const project = await import("/src/state/project.ts");
const sim = await import("/src/scene/simulation.ts");
const { quietly } = await import("/src/state/history.ts");
const { useTour } = await import("/src/state/tour.ts");
const { useWizard } = await import("/src/state/wizard.ts");
const pa = () => useApp.getState().objects.find((o) => o.kind === "pa" && o.visible);
const median = (values) => [...values].sort((a, b) => a - b)[Math.floor(values.length / 2)];
const nextFrame = () => new Promise(requestAnimationFrame);
useTour.setState({ mode: "off" });
useWizard.setState({ wizard: null });
localStorage.setItem("simrep.tour", "seen");
const rows = [];
for (const template of ["einzel", "quadrupole"]) {
  useApp.setState({ dirty: false });
  await project.newFromTemplate(template);
  if (pa()?.template !== template) throw new Error(`Template failed: ${template}`);
  const objectId = pa().id;
  for (const count of [10000, 100000]) {
    useApp.getState().updatePa(objectId, (o) => ({
      liveFly: false,
      source: { ...o.source, count, drawCount: 300, precision: "live" },
    }));
    // Actual completed warmup flights: no fixed delay guesses for GPU readiness.
    const deadline = performance.now() + 60000;
    let warmups = 0;
    do {
      await sim.flyNow(objectId);
      warmups++;
      if (pa().flight.status === "error") throw new Error(JSON.stringify(pa().flight.error));
      if (performance.now() > deadline) throw new Error("Hardware GPU preparation deadline");
    } while (pa().flight.result?.header.device.kind !== "gpu");
    for (let warm = 0; warm < 4; warm++) await sim.flyNow(objectId);
    useApp.getState().updatePa(objectId, () => ({ liveFly: true }));
    const values = { roundTrip: [], fly: [], paths: [], total: [] };
    const devices = new Set();
    let last = pa().flight.result;
    let frames = 0;
    let voltageChanges = 0;
    let previousVoltage = pa().voltages[2];
    let measuring = true;
    const unsubscribe = useApp.subscribe(() => {
      const result = pa()?.flight.result;
      if (!measuring || !result || result === last) return;
      last = result;
      values.roundTrip.push(result.roundTripMs);
      values.fly.push(result.header.flyMs);
      values.paths.push(result.header.pathsMs);
      values.total.push(result.header.totalMs);
      devices.add(`${result.header.device.kind}:${result.header.device.name}:${JSON.stringify(result.header.fallback)}`);
    });
    const from = template === "einzel" ? 60 : -3;
    const to = template === "einzel" ? 110 : -1;
    const start = await nextFrame();
    let now = start;
    while (now - start < 5000) {
      const fraction = (now - start) / 5000;
      const voltage = from + (to - from) * (0.5 - 0.5 * Math.cos(2 * Math.PI * fraction));
      if (voltage !== previousVoltage) voltageChanges++;
      previousVoltage = voltage;
      quietly(() => sim.setElectrodeVoltage(objectId, 2, voltage));
      frames++;
      now = await nextFrame();
    }
    measuring = false;
    unsubscribe();
    // A foreground fly awaits the same resident field after the final voltage change.
    useApp.getState().updatePa(objectId, () => ({ liveFly: false }));
    await sim.flyNow(objectId);
    if (!values.fly.length) throw new Error("No store results during voltage drag");
    rows.push({
      template, count, drawn: last.header.drawn, precision: last.header.precision,
      warmups, seconds: (now - start) / 1000, sliderEvents: frames, voltageChanges,
      updates: values.fly.length, updatesPerS: values.fly.length / ((now - start) / 1000),
      roundTripMedianMs: median(values.roundTrip), flyMedianMs: median(values.fly),
      pathsMedianMs: median(values.paths), nativeTotalMedianMs: median(values.total),
      devices: [...devices], source: pa().source, voltages: pa().voltages,
    });
  }
}
return { viewport: [innerWidth, innerHeight], rows };
