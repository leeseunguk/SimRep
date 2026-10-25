import { useEffect, useMemo, useState, type ReactNode } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { Box, Check, Cpu, FileInput, FolderOpen, Maximize2, Play, RotateCcw, Shapes, Sparkles, TriangleAlert, X, Zap } from "lucide-react";
import { useI18n, type Message, type MessageKey, type MessageParams } from "../i18n";
import { refusalLine, refusals, type Refusal } from "../refusals";
import { Chart } from "../results/plot/ChartView";
import { usePlotTheme } from "../results/plot/usePlotTheme";
import { BEAM_SHAPES, FLY_COLLISIONS, FLY_PRECISIONS, REFINE_DEVICES } from "../scene/options";
import { deviceLabel, reasonMessage, refineTargets, type BeamShape, type DeviceInfo, type FlyCollision, type FlyPrecision, type RefineDevice, type RefineTargets } from "../scene/backend";
import { electrodeName, paElectrodes, type PaElectrode } from "../scene/electrodes";
import { DEFAULT_DRAWN, MAX_DRAWN, MAX_IONS, type FlightHeader, type IonSource, type Surface } from "../scene/flight";
import type { Vec3 } from "../scene/mesh";
import { displayBounds, paKind } from "../scene/pa";
import { boundsToWorld, isIdentity, isRotated } from "../scene/placement";
import { importPaths } from "../scene/importFiles";
import {
  cancelRefine,
  flyBlocker,
  flyNow,
  resetVoltages,
  setElectrodeVoltage,
  simulationAvailable,
  startRefine,
  updateSource,
} from "../scene/simulation";
import { useApp, type PaObject, type RefineState } from "../state/store";
import { TOUR_ELECTRODE } from "../state/tour";
import { refineProgress, residualChart, stageLabel, surfaceLabel } from "./simulationFormat";
import { Badge } from "./ui/Badge";
import { Button } from "./ui/Button";
import { Field, ReadOnlyValue } from "./ui/Field";
import { NumberField } from "./ui/NumberField";
import { Section } from "./ui/Section";
import { SegmentedControl } from "./ui/SegmentedControl";
import { Switch } from "./ui/Switch";
import { Vec3Field } from "./ui/Vec3Field";
import inputs from "./ui/inputs.module.css";
import styles from "./SimulationSections.module.css";

/** Refine, electrode voltages, ion source + flight results for one potential array. */
export function SimulationSections({ o }: { o: PaObject }) {
  const canRefine = (o.formatId === "paHash" || o.formatId === "pa") && o.info.refinable;
  return (
    <>
      {(canRefine || o.refine) && <RefineSection o={o} canRefine={canRefine} />}
      {o.formatId === "pa0" && <VoltagesSection o={o} />}
      {o.formatId !== "paHash" && <IonsSection o={o} />}
      {o.formatId !== "paHash" && <WorkbenchSection o={o} />}
      {o.flight.result && <ResultsSection o={o} />}
    </>
  );
}

/** A device name in the UI language (CPU thread counts are localized). */
function deviceText(tm: (m: Message) => string, d: DeviceInfo): string {
  const label = deviceLabel(d);
  return typeof label === "string" ? label : tm(label);
}

/**
 * What actually flew (device, precision), named with the requested mode. A live request that ran
 * as CPU f64 (exact surfaces, no GPU) says so instead of showing the requested mode alone.
 * Lives here, not in the lazily loaded results panel, which also shows it.
 */
export function runText(t: (key: MessageKey, params?: MessageParams) => string, h: FlightHeader) {
  const ran = h.executed;
  const run = `${ran.device.toUpperCase()} ${ran.precision}`;
  const mode = t(h.precision === "live" ? "ions.precision.live" : "ions.precision.precise");
  const asRequested = h.precision === "live" ? ran.device === "gpu" && ran.precision === "f32" : ran.device === "cpu" && ran.precision === "f64";
  return { mode, run, asRequested, text: t(asRequested ? "results.run.as" : "results.run.instead", { mode, run }) };
}

export function RunBadge({ h }: { h: FlightHeader }) {
  const { t, tm } = useI18n();
  const ran = h.executed;
  const { mode, run, asRequested, text } = runText(t, h);
  const surfaces = t(ran.surfaces === "grid" ? "results.surfaces.grid" : h.surface?.accuracy === "source" ? "results.surfaces.exact" : "results.surfaces.approximate");
  const title = [t("results.run.title", { mode, run, surfaces }), h.fallback && tm(refusalLine(refusals.liveOnCpu(h.fallback)))].filter(Boolean).join("\n");
  return (
    <Badge tone={!asRequested ? "warning" : ran.precision === "f64" ? "success" : "info"} title={title}>
      {/* f32 / f64 keep their case inside the uppercase badge. */}
      <span className={styles.unit}>{text}</span>
    </Badge>
  );
}

export function Notice({ tone = "info", title, body, children }: { tone?: "info" | "warning" | "danger" | "success"; title: string; body?: string; children?: ReactNode }) {
  const Icon = tone === "success" ? Check : tone === "info" ? Sparkles : TriangleAlert;
  return (
    <div className={styles.notice} data-tone={tone}>
      <Icon className={styles.noticeIcon} aria-hidden />
      <div className={styles.noticeText}>
        <p className={styles.noticeTitle}>{title}</p>
        {body && <p className={styles.noticeBody}>{body}</p>}
        {children}
      </div>
    </div>
  );
}

// ------------------------------------------------------------------------------------ refine

/** A refusal as a notice: what happened as the title, why and what to do as the body. */
export function RefusalNotice({ refusal, tone = "info" }: { refusal: Refusal; tone?: "info" | "warning" }) {
  const { tm } = useI18n();
  return <Notice tone={tone} title={tm(refusal.what)} body={tm(refusal.why)} />;
}

/** Refine device choices (refine section and import wizard). */
export function deviceOptions(autoLabel: string) {
  return REFINE_DEVICES.map((d) => ({
    value: d,
    label: d === "auto" ? autoLabel : d.toUpperCase(),
    icon: d === "gpu" ? <Zap /> : d === "cpu" ? <Cpu /> : undefined,
  }));
}

/** Live log-scale convergence of the running refine (one point per V-cycle, see refine.rs). */
function ResidualChart({ r }: { r: RefineState }) {
  const { t } = useI18n();
  const theme = usePlotTheme();
  const tolerance = r.tolerance.toExponential(0);
  const spec = useMemo(
    () => residualChart(r, theme, { title: t("refine.chart.title"), x: t("refine.chart.x"), target: t("refine.chart.target", { tolerance }), earlier: t("refine.chart.earlier") }),
    [r.history, r.past, r.tolerance, theme, t],
  );
  const readout = (x: number) => {
    const i = Math.round(x);
    const v = r.history[i];
    return v === undefined ? null : [t("refine.cycle", { n: i }), v.toExponential(2)];
  };
  const residual = r.residual === null ? "…" : r.residual.toExponential(1);
  return (
    <div className={styles.residualChart}>
      <Chart spec={spec} ariaLabel={t("refine.chart.aria", { cycle: r.cycle, residual, tolerance })} readout={readout} />
    </div>
  );
}

function RefineSection({ o, canRefine }: { o: PaObject; canRefine: boolean }) {
  const { t, tm, num } = useI18n();
  const desktop = simulationAvailable();
  const [device, setDevice] = useState<RefineDevice>("auto");
  const [targets, setTargets] = useState<RefineTargets | null>(null);
  const [confirm, setConfirm] = useState(false);
  const r = o.refine;
  const running = r?.status === "running" || r?.status === "starting";

  useEffect(() => {
    if (!desktop || !canRefine || running) return;
    let live = true;
    refineTargets(o.id, o.outputDir).then(
      (x) => live && setTargets(x),
      () => live && setTargets(null),
    );
    return () => {
      live = false;
    };
  }, [desktop, canRefine, running, o.id, o.outputDir, o.path]);
  useEffect(() => setConfirm(false), [o.outputDir]);

  const existing = targets?.files.filter((f) => f.exists) ?? [];
  const start = () => {
    if (existing.length > 0 && !confirm) {
      setConfirm(true);
      return;
    }
    setConfirm(false);
    void startRefine(o.id, device);
  };
  const chooseFolder = async () => {
    const dir = await open({ directory: true, defaultPath: targets?.dir, title: t("refine.folder.pick") });
    if (typeof dir === "string") useApp.getState().updatePa(o.id, () => ({ outputDir: dir }));
  };

  return (
    <Section title={t("refine.title")}>
      {!desktop ? (
        <RefusalNotice refusal={refusals.desktopOnly()} />
      ) : running && r ? (
        <RefineProgress r={r} onCancel={() => cancelRefine(o.id)} />
      ) : (
        <>
          {r?.status === "done" && r.done && (
            <Notice
              tone="success"
              title={t("refine.done.title", { cycles: r.done.cycles, residual: r.done.residual.toExponential(1) })}
              body={t("refine.done.body", { ms: num(r.done.ms, 0), device: r.device ? deviceLabel(r.device) : "", files: r.done.files.join(", "), dir: r.done.dir })}
            />
          )}
          {r?.status === "cancelled" && <Notice tone="warning" title={t("refine.cancelled")} />}
          {r?.status === "error" && r.note && <Notice tone="danger" title={t("refine.failed")} body={tm(r.note)} />}
          {canRefine && (
            <>
              {o.formatId === "paHash" && !r && <p className={styles.lead}>{t("refine.lead.paHash")}</p>}
              {o.formatId === "pa" && !r && <p className={styles.lead}>{t("refine.lead.pa")}</p>}
              <Field label={t("refine.folder")} hint={o.outputDir ? undefined : t("refine.folder.sameHint")}>
                <span className={styles.path} title={targets?.dir}>
                  {targets?.dir ?? "…"}
                </span>
              </Field>
              <div className={styles.row}>
                <Button size="sm" icon={<FolderOpen />} onClick={() => void chooseFolder()}>
                  {t("refine.folder.change")}
                </Button>
                {o.outputDir && (
                  <Button size="sm" variant="ghost" icon={<RotateCcw />} onClick={() => useApp.getState().updatePa(o.id, () => ({ outputDir: null }))}>
                    {t("refine.folder.reset")}
                  </Button>
                )}
              </div>
              {targets && (
                <p className={styles.hint}>
                  {t("refine.writes", { files: targets.files.map((f) => f.name).join(", ") })}
                  {existing.length > 0 && <span className={styles.warnText}> {t("refine.replaces", { count: existing.length })}</span>}
                </p>
              )}
              <Field label={t("refine.device")} hint={t(`refine.device.${device}Hint`)}>
                <SegmentedControl<RefineDevice>
                  ariaLabel={t("refine.device")}
                  value={device}
                  onChange={setDevice}
                  options={deviceOptions(t("refine.device.auto"))}
                />
              </Field>
              {confirm ? (
                <div className={styles.confirm}>
                  <p>{t("refine.confirm", { count: existing.length, files: existing.map((f) => f.name).join(", ") })}</p>
                  <div className={styles.row}>
                    <Button size="sm" variant="primary" onClick={start}>
                      {t("refine.confirm.replace")}
                    </Button>
                    <Button size="sm" variant="ghost" onClick={() => setConfirm(false)}>
                      {t("refine.confirm.cancel")}
                    </Button>
                  </div>
                </div>
              ) : (
                <Button variant="primary" icon={<Sparkles />} className={styles.primary} onClick={start} disabled={!targets}>
                  {t(r?.status === "done" ? "refine.again" : "refine.start")}
                </Button>
              )}
            </>
          )}
          {!canRefine && o.formatId === "pa0" && r?.status === "done" && <p className={styles.hint}>{tm({ key: "refine.next" })}</p>}
        </>
      )}
    </Section>
  );
}

export function RefineProgress({ r, onCancel }: { r: RefineState; onCancel(): void }) {
  const { t, tm, num } = useI18n();
  const progress = refineProgress(r);
  return (
    <div className={styles.progress} aria-live="polite">
      <div className={styles.progressHead}>
        <span className={styles.progressTitle}>{r.stages.length ? tm(stageLabel(r)) : t("refine.starting")}</span>
        <span className="num">{num(progress * 100, 0)}%</span>
      </div>
      <div className={styles.meter}>
        <span style={{ width: `${Math.round(progress * 100)}%` }} />
      </div>
      <ResidualChart r={r} />
      <div className={styles.stats}>
        <span>{t("refine.cycle", { n: r.cycle })}</span>
        <span className="tnum" title={t("refine.residualHint")}>
          {t("refine.residual")} {r.residual === null ? "…" : r.residual.toExponential(2)}
        </span>
        <span className="num">{num(r.elapsedMs / 1000, 2)} s</span>
      </div>
      {r.device && (
        <p className={styles.hint}>
          {r.device.kind === "gpu" ? <Zap className={styles.inlineIcon} aria-hidden /> : <Cpu className={styles.inlineIcon} aria-hidden />} {deviceText(tm, r.device)}
        </p>
      )}
      {r.note && <p className={styles.warnHint}>{tm(refusalLine(refusals.refineOnCpu(r.note)))}</p>}
      <Button size="sm" icon={<X />} onClick={onCancel} disabled={r.job === null}>
        {t("refine.cancel")}
      </Button>
    </div>
  );
}

// ---------------------------------------------------------------------------------- voltages

/** 1, 2 or 5 x 10^k at or above v. */
function niceCeil(v: number): number {
  const p = 10 ** Math.floor(Math.log10(v));
  const m = v / p;
  return (m <= 1 ? 1 : m <= 2 ? 2 : m <= 5 ? 5 : 10) * p;
}

/**
 * Per array: the voltages its slider ranges are sized from while the user adjusts them with the
 * controls (`voltages`), and the voltages object the last such edit produced (`set`). Any other
 * change (undo, reset, a template, a project, a refine result) replaces the object, so the ranges
 * follow the new voltages. UI state only: not part of the document or the undo history.
 */
const sizedFrom = new Map<string, { voltages: Record<number, number>; set: Record<number, number> }>();

function sizingVoltages(o: PaObject): Record<number, number> {
  const pin = sizedFrom.get(o.id);
  return pin && pin.set === o.voltages ? pin.voltages : o.voltages;
}

/**
 * Slider half-range of an array's electrodes: about 4x the largest |V| among its file and current
 * voltages, rounded up to 1, 2 or 5 x 10^k and at least 1 V (a +-2.5 V quadrupole gets +-10 V, an
 * 80 V einzel lens +-500 V), and never narrower than `value`. While the user drags or types inside
 * the range it stays as it was; a value typed beyond it re-sizes the range from the new voltages.
 */
function sliderRange(o: PaObject, value: number): number {
  const volts = sizingVoltages(o);
  const scale = o.fast?.state === "ready" ? o.fast.electrodes.reduce((m, e) => Math.max(m, Math.abs(e.voltage), Math.abs(volts[e.number] ?? e.voltage)), 0) : 0;
  return niceCeil(Math.max(1, 4 * scale, Math.abs(value)));
}

function setFromControl(id: string, electrode: number, voltage: number, range: number) {
  const before = useApp.getState().objects.find((x): x is PaObject => x.id === id && x.kind === "pa");
  if (!before) return;
  const from = Math.abs(voltage) > range ? null : sizingVoltages(before);
  setElectrodeVoltage(id, electrode, voltage);
  const after = useApp.getState().objects.find((x): x is PaObject => x.id === id && x.kind === "pa");
  if (from && after) sizedFrom.set(id, { voltages: from, set: after.voltages });
  else sizedFrom.delete(id);
}

/** Voltage field + slider of one adjustable electrode. `tour`: anchor for the guided tour (components/Tour.tsx). */
export function VoltageControl({ o, electrode, label, tour }: { o: PaObject; electrode: PaElectrode; label: string; tour?: string }) {
  const { t, num } = useI18n();
  const value = electrode.voltage ?? 0;
  const range = sliderRange(o, value);
  const onChange = (v: number) => setFromControl(o.id, electrode.number!, v, range);
  // 2000 slider positions; the field shows the step's decimals (0.01 V on +-10 V).
  const step = range / 1000;
  const digits = Math.max(0, Math.ceil(-Math.log10(step) - 1e-9), Number.isInteger(value) ? 0 : 1);
  const widen = t("volts.widenHint", { range: num(range) });
  return (
    <div className={styles.voltage} data-tour={tour}>
      <NumberField label={label} unit={t("units.V")} value={value} step={step} precision={digits} scrubTitle={t("props.scrubHint")} onChange={onChange} />
      <input
        type="range"
        className={inputs.slider}
        min={-range}
        max={range}
        step={step}
        value={value}
        aria-label={label}
        title={widen}
        onChange={(e) => onChange(Number(e.target.value))}
      />
      <div className={styles.scale} title={widen}>
        <span className="num">−{num(range)}</span>
        <span className="num">0</span>
        <span className="num">+{num(range)}</span>
      </div>
    </div>
  );
}

function VoltagesSection({ o }: { o: PaObject }) {
  const { t, num } = useI18n();
  const fast = o.fast;
  const electrodes = paElectrodes(o);
  return (
    <Section title={t("volts.title")}>
      {!simulationAvailable() ? (
        <RefusalNotice refusal={refusals.desktopOnly()} />
      ) : fast?.state !== "ready" ? (
        <Notice tone="warning" title={t("volts.missing.title")} body={t("volts.missing.body", { detail: fast?.detail ?? "" })} />
      ) : (
        <>
          <p className={styles.lead}>{t("volts.lead")}</p>
          {fast.electrodes.every((e) => (o.voltages[e.number] ?? e.voltage) === 0) && <Notice title={t("volts.allZero.title")} body={t("volts.allZero.body")} />}
          {electrodes
            .filter((e) => e.kind === "adjustable")
            .map((e) => (
              <VoltageControl
                key={e.label}
                o={o}
                electrode={e}
                label={e.number ? `#${e.number} ${electrodeName(o, e, t, num)}` : electrodeName(o, e, t, num)}
                tour={o.template === "einzel" && e.number === TOUR_ELECTRODE ? `voltage-${TOUR_ELECTRODE}` : undefined}
              />
            ))}
          <Field label={t("volts.live")} hint={t("volts.liveHint")}>
            <Switch checked={o.liveFly} label={t("volts.live")} onChange={(liveFly) => useApp.getState().updatePa(o.id, () => ({ liveFly }))} />
          </Field>
          <div className={styles.row}>
            <Button size="sm" variant="ghost" icon={<RotateCcw />} onClick={() => resetVoltages(o.id)}>
              {t("volts.reset")}
            </Button>
          </div>
        </>
      )}
    </Section>
  );
}

// --------------------------------------------------------------------------------- workbench

type Face = Extract<Surface, { kind: "wall" }>["face"];
const round4 = (v: number) => Number(v.toPrecision(4));

/** The array's own box on the workbench (where ions stop without a workbench box). */
function arrayBox(o: PaObject): { min: Vec3; max: Vec3 } {
  return boundsToWorld(o.source.bench.placement, displayBounds(o.info, true));
}

/**
 * A workbench box that lets ions leave the array: the array box, extended by twice the array's
 * size beyond each face ions crossed (SIMION's einzel workbench reaches about 2.2 lengths past the
 * lens exit). Without hit faces, along the beam direction.
 */
export function suggestBench(o: PaObject, faces: Face[]): { min: Vec3; max: Vec3 } {
  const b = arrayBox(o);
  const min = [...b.min] as Vec3;
  const max = [...b.max] as Vec3;
  const size = [0, 1, 2].map((a) => Math.max(b.max[a]! - b.min[a]!, 1));
  const list: Face[] = faces.length ? faces : ["+x"];
  for (const f of list) {
    if (f === "r") {
      for (const a of [1, 2]) {
        min[a] = min[a]! - size[a]!;
        max[a] = max[a]! + size[a]!;
      }
      continue;
    }
    const a = "xyz".indexOf(f[1]!);
    if (f[0] === "+") max[a] = max[a]! + 2 * size[a]!;
    else min[a] = min[a]! - 2 * size[a]!;
  }
  return { min: min.map(round4) as Vec3, max: max.map(round4) as Vec3 };
}

const sameBox = (a: { min: Vec3; max: Vec3 }, b: { min: Vec3; max: Vec3 }) => a.min.every((v, i) => Math.abs(v - b.min[i]!) < 1e-9) && a.max.every((v, i) => Math.abs(v - b.max[i]!) < 1e-9);

/** Ions that ended on the array's own edge (no workbench box), by face. */
export function edgeStops(o: PaObject): { count: number; faces: Face[] } {
  const h = o.flight.result?.header;
  if (!h || o.source.bench.enabled) return { count: 0, faces: [] };
  const walls = h.surfaces.filter((s): s is typeof s & { surface: { kind: "wall"; face: Face } } => s.surface.kind === "wall" && s.count > 0);
  return { count: walls.reduce((n, s) => n + s.count, 0), faces: walls.map((s) => s.surface.face) };
}

function WorkbenchSection({ o }: { o: PaObject }) {
  const { t, num } = useI18n();
  const s = o.source;
  const bench = s.bench;
  const place = bench.placement;
  const setBench = (patch: Partial<IonSource["bench"]>) => updateSource(o.id, { bench: { ...bench, ...patch } });
  const toggle = (enabled: boolean) => {
    // The default box is the array itself (no change in flight): open it along the beam instead.
    if (enabled && sameBox(bench, arrayBox(o))) setBench({ enabled, ...suggestBench(o, edgeStops(o).faces) });
    else setBench({ enabled });
  };
  const openIob = async () => {
    const path = await open({ title: t("bench.iob.pick"), filters: [{ name: t("bench.iob.filter"), extensions: ["iob"] }] });
    if (typeof path === "string") await importPaths([path]);
  };
  return (
    <Section title={t("bench.title")}>
      <Field label={t("ions.bench")} hint={t("ions.benchHint")}>
        <Switch checked={bench.enabled} label={t("ions.bench")} onChange={toggle} />
      </Field>
      {bench.enabled ? (
        <>
          <Vec3Field label={t("ions.bench.min")} unit="mm" value={bench.min} onChange={(min) => setBench({ min })} />
          <Vec3Field label={t("ions.bench.max")} unit="mm" value={bench.max} onChange={(max) => setBench({ max })} />
          <div className={styles.row}>
            <Button size="sm" variant="ghost" icon={<Maximize2 />} onClick={() => setBench(suggestBench(o, edgeStops(o).faces))} title={t("bench.extendHint")}>
              {t("bench.extend")}
            </Button>
            <Button size="sm" variant="ghost" icon={<Box />} onClick={() => setBench(arrayBox(o))}>
              {t("bench.fitArray")}
            </Button>
          </div>
          <p className={styles.hint}>{t(bench.from ? "bench.fromIob" : "bench.drawn", { file: bench.from ?? "" })}</p>
        </>
      ) : (
        <p className={styles.warnHint}>{t("bench.off")}</p>
      )}
      {!isIdentity(place) && place && (
        <div className={styles.placement}>
          <p className={styles.placementTitle}>{t("bench.placement")}</p>
          <p className={`num ${styles.hint}`}>
            {t("bench.placementValue", { x: num(place.origin[0], 3), y: num(place.origin[1], 3), z: num(place.origin[2], 3), scale: num(place.scale, 5) })}
            {isRotated(place) && ` · ${t("bench.rotated")}`}
          </p>
        </div>
      )}
      <div className={styles.row}>
        <Button size="sm" icon={<FileInput />} onClick={() => void openIob()} disabled={!simulationAvailable()}>
          {t("bench.iob")}
        </Button>
      </div>
    </Section>
  );
}

// -------------------------------------------------------------------------------------- ions

const ION_PRESETS = [1_000, 10_000, 100_000];

function IonsSection({ o }: { o: PaObject }) {
  const { t, tm, lang } = useI18n();
  const compute = useApp((st) => st.compute);
  const compact = new Intl.NumberFormat(lang === "ko" ? "ko-KR" : "en-US", { notation: "compact" });
  const s = o.source;
  const set = (patch: Partial<IonSource>) => updateSource(o.id, patch);
  const blocker = flyBlocker(o);
  const running = o.flight.status === "running";
  const scrub = t("props.scrubHint");
  const chooseFile = async () => {
    const path = await open({ title: t("ions.file.pick"), filters: [{ name: t("ions.file.filter"), extensions: ["ion", "fly2"] }] });
    if (typeof path === "string") set({ mode: "file", file: { path, name: path.split(/[\\/]/).pop() ?? path } });
  };
  const cylindrical = paKind(o.info) === "cylindrical";
  const precision = s.precision ?? "live";
  const drawn = s.drawCount ?? DEFAULT_DRAWN;
  // Exact surfaces apply only to an array with a .cuts.json next to it (import wizard cut cells).
  const cuts = o.info.cuts ?? null;
  const exact = cuts !== null && (s.exactSurfaces ?? true);
  const used = o.flight.result?.header.surface ?? null;
  // File sources: the count is known once flown.
  const ions = s.mode === "beam" ? s.count : (o.flight.result?.header.ions ?? drawn);

  return (
    <Section title={t("ions.title")}>
      <Field label={t("ions.source")}>
        <SegmentedControl<IonSource["mode"]>
          ariaLabel={t("ions.source")}
          value={s.mode}
          onChange={(mode) => set({ mode })}
          options={[
            { value: "beam", label: t("ions.source.beam") },
            { value: "file", label: t("ions.source.file") },
          ]}
        />
      </Field>
      {s.mode === "beam" ? (
        <>
          <NumberField label={t("ions.count")} value={s.count} step={10} precision={0} min={1} max={MAX_IONS} scrubTitle={scrub} onChange={(count) => set({ count: Math.round(count) })} />
          <div className={styles.presets} role="group" aria-label={t("ions.presets")}>
            {ION_PRESETS.map((count) => (
              <Button key={count} size="sm" variant="ghost" active={s.count === count} onClick={() => set({ count })}>
                {compact.format(count)}
              </Button>
            ))}
          </div>
          <NumberField label={t("ions.mass")} unit="amu" value={s.massAmu} step={1} precision={3} min={1e-6} scrubTitle={scrub} onChange={(massAmu) => set({ massAmu })} />
          <NumberField label={t("ions.charge")} unit="e" value={s.chargeE} step={1} precision={0} scrubTitle={scrub} onChange={(chargeE) => set({ chargeE })} />
          <NumberField label={t("ions.energy")} unit="eV" value={s.energyEv} step={1} precision={1} min={0} scrubTitle={scrub} onChange={(energyEv) => set({ energyEv })} />
          <Vec3Field label={t("ions.position")} unit="mm" value={s.position} step={0.5} onChange={(position) => set({ position })} />
          <NumberField label={t("ions.azimuth")} unit="°" value={s.azimuthDeg} step={1} precision={1} scrubTitle={scrub} onChange={(azimuthDeg) => set({ azimuthDeg })} />
          <NumberField label={t("ions.elevation")} unit="°" value={s.elevationDeg} step={1} precision={1} min={-90} max={90} scrubTitle={scrub} onChange={(elevationDeg) => set({ elevationDeg })} />
          <Field label={t("ions.shape")} hint={t(`ions.shape.${s.shape}Hint`)}>
            <SegmentedControl<BeamShape>
              ariaLabel={t("ions.shape")}
              value={s.shape}
              onChange={(shape) => set({ shape })}
              options={BEAM_SHAPES.map((v) => ({ value: v, label: t(`ions.shape.${v}`) }))}
            />
          </Field>
          {s.shape !== "point" && (
            <NumberField label={t(s.shape === "line" ? "ions.width.line" : "ions.width.disk")} unit="mm" value={s.widthMm} step={0.5} precision={2} min={0} scrubTitle={scrub} onChange={(widthMm) => set({ widthMm })} />
          )}
          <NumberField label={t("ions.cone")} unit="°" value={s.coneDeg} step={0.5} precision={1} min={0} max={180} scrubTitle={scrub} onChange={(coneDeg) => set({ coneDeg })} />
        </>
      ) : (
        <>
          <Field label={t("ions.file")}>
            <ReadOnlyValue>{s.file?.name ?? t("ions.file.none")}</ReadOnlyValue>
          </Field>
          <div className={styles.row}>
            <Button size="sm" icon={<FileInput />} onClick={() => void chooseFile()} disabled={!simulationAvailable()}>
              {t("ions.file.choose")}
            </Button>
          </div>
          <p className={styles.hint}>{t("ions.file.hint")}</p>
        </>
      )}
      {cuts && (
        <div className={styles.surfaceCard} data-on={exact}>
          <Shapes className={styles.surfaceIcon} aria-hidden />
          <div className={styles.surfaceText}>
            <span className={styles.surfaceName}>
              {t("ions.surfaces")}
              {exact && used && <Badge tone={used.accuracy === "source" ? "success" : "warning"}>{t(used.accuracy === "source" ? "ions.surfaces.exact" : "ions.surfaces.approximate")}</Badge>}
            </span>
            <span className={styles.surfaceFile} title={cuts}>
              {cuts}
            </span>
          </div>
          <Switch checked={exact} label={t("ions.surfaces")} onChange={(exactSurfaces) => set({ exactSurfaces })} />
          <p className={styles.surfaceHint}>{t(exact ? "ions.surfaces.on" : "ions.surfaces.off", { file: cuts })}</p>
        </div>
      )}
      <div className={styles.modes} role="radiogroup" aria-label={t("ions.precision")}>
        <span className={styles.modesLabel}>{t("ions.precision")}</span>
        {FLY_PRECISIONS.map((p: FlyPrecision) => {
          // Live flies as CPU f64 when surfaces are checked exactly or there is no GPU; the request stays Live.
          const cpuLive = p === "live" && (exact || compute.state === "cpu");
          return (
            <button key={p} type="button" role="radio" aria-checked={precision === p} className={styles.mode} data-cpu={cpuLive} onClick={() => set({ precision: p })}>
              {p === "live" ? <Zap className={styles.modeIcon} aria-hidden /> : <Cpu className={styles.modeIcon} aria-hidden />}
              <span className={styles.modeName}>{t(`ions.precision.${p}`)}</span>
              <span className={styles.modeSub}>{t(cpuLive ? "ions.precision.liveCpuSub" : `ions.precision.${p}Sub`)}</span>
            </button>
          );
        })}
        <p className={styles.modesHint}>
          {precision === "live" && exact
            ? tm(reasonMessage({ code: "exactSurfaces" }))
            : t(precision === "precise" ? "ions.precision.preciseHint" : compute.state === "cpu" ? "ions.precision.liveCpuHint" : "ions.precision.liveHint")}
        </p>
      </div>
      <NumberField label={t("ions.drawn")} value={drawn} step={50} precision={0} min={0} max={MAX_DRAWN} scrubTitle={scrub} onChange={(drawCount) => set({ drawCount: Math.round(drawCount) })} />
      <p className={styles.hint}>{t("ions.drawnHint", { drawn: Math.min(drawn, ions), ions })}</p>
      <details className={styles.advanced}>
        <summary>{t("ions.advanced")}</summary>
        <div className={styles.advancedBody}>
          <NumberField label={t("ions.duration")} unit="µs" value={s.durationUs} step={10} precision={1} min={0.001} scrubTitle={scrub} onChange={(durationUs) => set({ durationUs })} />
          <Field label={t("ions.collision")} hint={t(exact ? "ions.collision.exactHint" : `ions.collision.${s.collision ?? "allCorners"}Hint`)}>
            <SegmentedControl<FlyCollision>
              ariaLabel={t("ions.collision")}
              value={s.collision ?? "allCorners"}
              onChange={(collision) => set({ collision })}
              options={FLY_COLLISIONS.map((v) => ({ value: v, label: t(`ions.collision.${v}`) }))}
            />
          </Field>
          {cylindrical && <p className={styles.hint}>{t("ions.cylindricalHint")}</p>}
        </div>
      </details>
      {blocker && <p className={styles.blocker}>{tm(refusalLine(refusals.flyBlocked(blocker)))}</p>}
      <Button variant="primary" icon={<Play />} className={styles.primary} data-tour="fly" disabled={!!blocker || running} onClick={() => void flyNow(o.id)}>
        {t(running ? "ions.flying" : o.flight.result ? "ions.flyAgain" : "ions.fly")}
      </Button>
      {o.flight.status === "error" && o.flight.error && <Notice tone="danger" title={t("ions.failed")} body={tm(o.flight.error)} />}
    </Section>
  );
}

// ----------------------------------------------------------------------------------- results

function ResultsSection({ o }: { o: PaObject }) {
  const { t, tm, num } = useI18n();
  const f = o.flight.result!;
  const h = f.header;
  const sorted = [...h.surfaces].sort((a, b) => b.count - a.count);
  const tofDigits = h.tof.max >= 100 ? 1 : h.tof.max >= 1 ? 3 : 4;
  const ms = (v: number) => num(v, v < 10 ? 1 : 0);
  // Compute is the headline; preparation and display work around it are secondary.
  const prep = [
    h.prepareMs > 0 && t("results.prep.upload", { ms: ms(h.prepareMs) }),
    h.transferMs > 0 && t("results.prep.transfer", { ms: ms(h.transferMs) }),
    h.drawn > 0 && (h.pathsGpu ? t("results.prep.pathsGpu") : t("results.prep.paths", { ms: ms(h.pathsMs) })),
    t("results.prep.total", { ms: ms(f.roundTripMs) }),
  ]
    .filter(Boolean)
    .join(" · ");
  const edge = edgeStops(o);
  return (
    <Section title={t("results.title")}>
      {edge.count > 0 && (
        <Notice tone="warning" title={t("results.edge.title", { n: num(edge.count), ions: num(h.ions) })} body={t("results.edge.body")}>
          <div className={styles.row}>
            <Button size="sm" variant="primary" icon={<Maximize2 />} onClick={() => updateSource(o.id, { bench: { ...o.source.bench, enabled: true, ...suggestBench(o, edge.faces) } })}>
              {t("results.edge.add")}
            </Button>
          </div>
        </Notice>
      )}
      <dl className={styles.meta}>
        <div>
          <dt>{t("results.mode")}</dt>
          <dd>
            <RunBadge h={h} />
          </dd>
        </div>
        {(h.surface || o.info.cuts) && (
          <div>
            <dt>{t("results.surfaces")}</dt>
            <dd>
              {h.surface ? (
                h.surface.accuracy === "source" ? (
                  <Badge tone="success" title={t("results.surfaces.exactTitle", { file: h.surface.file })}>
                    {t("results.surfaces.exact")}
                  </Badge>
                ) : (
                  <Badge tone="warning" title={t("results.surfaces.approximateTitle", { file: h.surface.file })}>
                    {t("results.surfaces.approximate")}
                  </Badge>
                )
              ) : (
                <Badge tone="neutral" title={t("results.surfaces.gridTitle")}>
                  {t("results.surfaces.grid")}
                </Badge>
              )}
            </dd>
          </div>
        )}
        <div>
          <dt>{t("results.device")}</dt>
          <dd title={deviceText(tm, h.device)}>
            {h.device.kind === "gpu" ? <Zap className={styles.inlineIcon} aria-hidden /> : <Cpu className={styles.inlineIcon} aria-hidden />} {deviceText(tm, h.device)}
          </dd>
        </div>
        <div>
          <dt>{t("results.count")}</dt>
          <dd className="tnum" title={h.thinned ? t("results.thinned") : undefined}>
            {t("results.countValue", { ions: h.ions, drawn: h.drawn })}
          </dd>
        </div>
        <div>
          <dt>{t("results.compute")}</dt>
          <dd className={`num ${styles.metaStrong}`}>{num(h.flyMs, h.flyMs < 10 ? 1 : 0)} ms</dd>
        </div>
        <div>
          <dt>{t("results.prep")}</dt>
          <dd className={`tnum ${styles.subtle} ${styles.metaWrap}`}>{prep}</dd>
        </div>
      </dl>
      {h.fallback && <p className={styles.warnHint}>{tm(refusalLine(refusals.liveOnCpu(h.fallback)))}</p>}
      {!h.fallback && <p className={styles.hint}>{t(h.executed.precision === "f32" ? "results.liveNote" : "results.preciseNote")}</p>}
      <Field label={t("results.show")}>
        <Switch checked={o.showTrajectories} label={t("results.show")} onChange={(showTrajectories) => useApp.getState().updatePa(o.id, () => ({ showTrajectories }))} />
      </Field>
      <div className={styles.hits} role="table" aria-label={t("results.hits")}>
        <div className={styles.hitsHead} role="row">
          <span role="columnheader">{t("results.hits")}</span>
          <span role="columnheader">{t("results.ions")}</span>
          <span role="columnheader">
            {t("results.tofMean")} <span className={styles.unit}>µs</span>
          </span>
        </div>
        {sorted.map((s, i) => (
          <div key={i} className={styles.hit} role="row" data-kind={s.surface.kind}>
            <span className={styles.hitBar} style={{ width: `${(100 * s.count) / h.ions}%` }} aria-hidden />
            <span role="cell" className={styles.hitName}>
              <i className={styles.hitDot} aria-hidden />
              {tm(surfaceLabel(s.surface))}
            </span>
            <span role="cell" className="num">
              {num(s.count)} <span className={styles.subtle}>({num((100 * s.count) / h.ions, 0)}%)</span>
            </span>
            <span role="cell" className="num">
              {s.surface.kind === "timeLimit" ? "—" : num(s.tofMean, tofDigits)}
            </span>
          </div>
        ))}
      </div>
      {h.tof.n > 0 && (
        <div className={styles.tof}>
          <p className={styles.tofTitle}>{t("results.tof", { n: h.tof.n })}</p>
          <dl className={styles.tofGrid}>
            {(["min", "mean", "max", "std"] as const).map((k) => (
              <div key={k}>
                <dt>{t(`results.tof.${k}`)}</dt>
                <dd className="num">{num(h.tof[k], tofDigits)}</dd>
              </div>
            ))}
          </dl>
        </div>
      )}
      <Field label={t("results.keMean")}>
        <ReadOnlyValue mono>{num(h.keMean, 2)} eV</ReadOnlyValue>
      </Field>
    </Section>
  );
}
