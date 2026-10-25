const { useWizard } = await import('/src/state/wizard.ts');
useWizard.getState().set({ step: __args.step }); await new Promise(r=>setTimeout(r,800));
const w = useWizard.getState().wizard; return { step: w.step, method: w.method, spacing: w.spacing, origin: w.origin, size: w.size, outputDir: w.outputDir, stem: w.stem, sidecar: w.sidecar, device: w.device };