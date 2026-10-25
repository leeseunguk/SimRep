import { useEffect, useState } from "react";
import { documentDir, join } from "@tauri-apps/api/path";
import { save } from "@tauri-apps/plugin-dialog";
import { Activity, Box, Check, Database, Download, FolderOpen, Layers, Waves, X } from "lucide-react";
import { useI18n, type Message } from "../i18n";
import type { MessageKey } from "../i18n/en";
import { backendError, errorMessage, exportArray, exportTrajectories, type ExportWhat, type SurfaceOptions } from "../scene/backend";
import { electrodeName, paElectrodes } from "../scene/electrodes";
import { importPaths } from "../scene/importFiles";
import { paKind } from "../scene/pa";
import { isIdentity, isRotated } from "../scene/placement";
import { flyRequest } from "../scene/simulation";
import { dirname } from "../state/project";
import { useApp, type PaObject } from "../state/store";
import { useExport, type ExportKind } from "../state/exportDialog";
import { Notice } from "./SimulationSections";
import { Button } from "./ui/Button";
import { Field } from "./ui/Field";
import { NumberField } from "./ui/NumberField";
import { SegmentedControl } from "./ui/SegmentedControl";
import { Switch } from "./ui/Switch";
import dialog from "./Dialog.module.css";
import sim from "./SimulationSections.module.css";
import styles from "./ExportDialog.module.css";

type MeshFormat = "stl" | "obj" | "gltf" | "glb";
type PaFormat = "pa" | "paHash" | "pa0";

const KINDS: readonly { kind: ExportKind; icon: typeof Box }[] = [
  { kind: "geometry", icon: Box },
  { kind: "grid", icon: Database },
  { kind: "isosurface", icon: Layers },
  { kind: "trajectories", icon: Activity },
  { kind: "pa", icon: Waves },
];

const MESH_FORMATS: readonly MeshFormat[] = ["stl", "obj", "gltf", "glb"];
const PA_EXT: Record<PaFormat, string> = { pa: "pa", paHash: "pa#", pa0: "pa0" };

const stemOf = (o: PaObject) => o.fileName.replace(/\.[^.]+$/, "");
const voltageList = (o: PaObject): [number, number][] => (o.fast?.state === "ready" ? o.fast.electrodes.map((e) => [e.number, o.voltages[e.number] ?? e.voltage]) : []);

/** A round potential halfway between the array's extremes (the default isopotential). */
function middlePotential(o: PaObject): number {
  const mid = (o.info.potentialMin + o.info.potentialMax) / 2;
  const span = Math.abs(o.info.potentialMax - o.info.potentialMin) || 1;
  const step = 10 ** Math.floor(Math.log10(span / 10));
  return Math.round(mid / step) * step;
}

/** Project folder, else the array's folder, unless that is a read-only install folder. */
async function defaultDir(o: PaObject): Promise<string | null> {
  const app = useApp.getState();
  const dir = app.project.dir ?? (app.project.path ? dirname(app.project.path) : null) ?? (o.path ? dirname(o.path) : null);
  if (dir && !/[\\/]program files( \(x86\))?([\\/]|$)/i.test(dir)) return dir;
  return join(await documentDir(), "SimRep");
}

/** Why a kind cannot be exported for this array (null = it can). */
function unavailable(o: PaObject, kind: ExportKind): MessageKey | null {
  const unsolved = o.formatId === "paHash";
  if ((kind === "grid" || kind === "isosurface") && unsolved) return "export.unavailable.unsolved";
  if (kind === "trajectories" && !o.flight.result) return "export.unavailable.noFlight";
  return null;
}

export function ExportDialog() {
  const { t, tm, num } = useI18n();
  const objectId = useExport((s) => s.objectId);
  const kind = useExport((s) => s.kind);
  const o = useApp((s) => s.objects.find((x) => x.id === objectId));
  // Select the list itself (stable) and filter here: a filtered array per store read would loop.
  const objects = useApp((s) => s.objects);
  const arrays = objects.filter((x): x is PaObject => x.kind === "pa");
  const close = useExport((s) => s.close);
  const [meshFormat, setMeshFormat] = useState<MeshFormat>("stl");
  const [segments, setSegments] = useState(64);
  const [sweep, setSweep] = useState(360);
  const [depth, setDepth] = useState<number | null>(null);
  const [field, setField] = useState(true);
  const [revolve, setRevolve] = useState(false);
  const [potential, setPotential] = useState<number | null>(null);
  const [trajFormat, setTrajFormat] = useState<"vtp" | "csv">("vtp");
  const [paFormat, setPaFormat] = useState<PaFormat | null>(null);
  const [busy, setBusy] = useState(false);
  const [done, setDone] = useState<{ files: string[]; dir: string; path: string; pa: boolean } | null>(null);
  const [error, setError] = useState<Message | null>(null);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && !busy && close();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [busy, close]);
  useEffect(() => {
    setDone(null);
    setError(null);
  }, [kind, objectId]);

  if (!o || o.kind !== "pa") return null;
  const shape = paKind(o.info);
  const cylindrical = shape === "cylindrical";
  const planar2d = shape === "planar2d";
  const extrusion = depth ?? Math.max(1, Number((o.info.ny * o.info.dy * 0.6).toPrecision(2)));
  const iso = potential ?? middlePotential(o);
  const pa: PaFormat = paFormat ?? (o.fast?.state === "ready" ? "pa0" : o.formatId === "paHash" ? "paHash" : "pa");
  const surface: SurfaceOptions = { angularSegments: segments, sweepDegrees: sweep, extrusionDepthMm: extrusion };
  const blocked = unavailable(o, kind);
  const placement = isIdentity(o.source.bench.placement) ? null : o.source.bench.placement!;
  const stem = stemOf(o);
  const gridExt = cylindrical && revolve ? "vts" : "vti";

  const target = (): { name: string; ext: string; filter: MessageKey } => {
    switch (kind) {
      case "geometry":
        return { name: `${stem}-electrodes.${meshFormat}`, ext: meshFormat, filter: `export.filter.${meshFormat}` };
      case "grid":
        return { name: `${stem}-field.${gridExt}`, ext: gridExt, filter: `export.filter.${gridExt}` };
      case "isosurface":
        return { name: `${stem}-iso-${num(iso).replace(/[^\d.-]/g, "")}V.${meshFormat}`, ext: meshFormat, filter: `export.filter.${meshFormat}` };
      case "trajectories":
        return { name: `${stem}-trajectories.${trajFormat}`, ext: trajFormat, filter: `export.filter.${trajFormat}` };
      case "pa":
        return { name: `${stem}-export.${PA_EXT[pa]}`, ext: PA_EXT[pa], filter: `export.filter.${pa}` };
    }
  };

  const run = async () => {
    const tg = target();
    const dir = await defaultDir(o);
    const sep = dir?.includes("\\") ? "\\" : "/";
    const path = await save({ title: t("export.saveTitle"), defaultPath: dir ? `${dir}${sep}${tg.name}` : tg.name, filters: [{ name: t(tg.filter), extensions: [tg.ext] }] });
    if (!path) return;
    setBusy(true);
    setError(null);
    setDone(null);
    const app = useApp.getState();
    try {
      let files: string[];
      if (kind === "trajectories") {
        const n = await exportTrajectories(flyRequest(o), path);
        files = [path.split(/[\\/]/).pop()!];
        app.log("info", { key: "log.exportTrajectories", params: { n } });
      } else {
        const names: [number, string][] = paElectrodes(o)
          .filter((e) => e.number !== null)
          .map((e) => [e.number!, electrodeName(o, e, t, num)]);
        const what: ExportWhat =
          kind === "geometry"
            ? { kind: "geometry", surface, names }
            : kind === "grid"
              ? { kind: "grid", field, angularSegments: segments, sweepDegrees: sweep }
              : kind === "isosurface"
                ? { kind: "isosurface", potential: iso, surface }
                : { kind: "pa", format: pa };
        files = await exportArray({ id: o.id, path, voltages: voltageList(o), placement, what });
      }
      const outDir = dirname(path);
      setDone({ files, dir: outDir, path, pa: kind === "pa" });
      app.log("success", { key: "log.exportedFiles", params: { files: files.join(", "), dir: outDir } });
      app.toast("success", { key: "toast.exported.title" }, { key: "toast.exportedFiles.body", params: { files: files.join(", "), dir: outDir } });
    } catch (err) {
      const detail = errorMessage(backendError(err));
      setError(detail);
      app.log("error", { key: "log.exportFailed", params: { path, detail } });
    } finally {
      setBusy(false);
    }
  };

  const reopen = async () => {
    if (!done) return;
    const [id] = await importPaths([done.path]);
    if (id) close();
  };

  const surfaceControls = (
    <>
      {cylindrical && (
        <>
          <Field label={t("export.sweep")} hint={t("export.sweepHint")}>
            <SegmentedControl<string>
              ariaLabel={t("export.sweep")}
              value={sweep === 360 ? "full" : sweep === 270 ? "cutaway" : "custom"}
              onChange={(v) => setSweep(v === "full" ? 360 : v === "cutaway" ? 270 : 180)}
              options={[
                { value: "full", label: t("export.sweep.full") },
                { value: "cutaway", label: t("export.sweep.cutaway") },
                { value: "custom", label: t("export.sweep.custom") },
              ]}
            />
          </Field>
          {sweep !== 360 && sweep !== 270 && (
            <NumberField label={t("export.sweepDeg")} unit="°" value={sweep} step={15} precision={0} min={1} max={360} onChange={(v) => setSweep(Math.round(v))} />
          )}
          <NumberField label={t("export.segments")} value={segments} step={8} precision={0} min={3} max={1024} onChange={(v) => setSegments(Math.round(v))} />
        </>
      )}
      {planar2d && <NumberField label={t("export.depth")} unit="mm" value={extrusion} step={1} precision={2} min={0.001} onChange={setDepth} />}
    </>
  );

  return (
    <div className={dialog.scrim} onPointerDown={(e) => e.target === e.currentTarget && !busy && close()}>
      <div className={`${dialog.dialog} ${styles.dialog}`} role="dialog" aria-modal="true" aria-labelledby="export-title">
        <div className={dialog.header}>
          <Download className={dialog.headerIcon} aria-hidden />
          <h2 id="export-title" className={dialog.title}>
            {t("export.title")}
          </h2>
          <button type="button" className={dialog.close} aria-label={t("export.close")} onClick={close} disabled={busy}>
            <X />
          </button>
        </div>
        {arrays.length > 1 && (
          <Field label={t("export.array")}>
            <select className={styles.select} value={o.id} onChange={(e) => useExport.getState().open(e.target.value)}>
              {arrays.map((a) => (
                <option key={a.id} value={a.id}>
                  {a.customName ?? a.fileName}
                </option>
              ))}
            </select>
          </Field>
        )}
        <div className={styles.body}>
          <nav className={styles.kinds} aria-label={t("export.what")}>
            {KINDS.map(({ kind: k, icon: Icon }) => (
              <button key={k} type="button" className={styles.kind} aria-pressed={kind === k} data-off={unavailable(o, k) !== null} onClick={() => useExport.getState().open(o.id, k)}>
                <Icon className={styles.kindIcon} aria-hidden />
                <span className={styles.kindText}>
                  <span className={styles.kindName}>{t(`export.kind.${k}`)}</span>
                  <span className={styles.kindSub}>{t(`export.kind.${k}Sub`)}</span>
                </span>
              </button>
            ))}
          </nav>
          <div className={styles.options}>
            <p className={sim.lead}>{t(`export.lead.${kind}`)}</p>
            {blocked ? (
              <Notice tone="warning" title={t(blocked)} />
            ) : (
              <>
                {(kind === "geometry" || kind === "isosurface") && (
                  <Field label={t("export.format")}>
                    <SegmentedControl<MeshFormat> ariaLabel={t("export.format")} value={meshFormat} onChange={setMeshFormat} options={MESH_FORMATS.map((f) => ({ value: f, label: f.toUpperCase() }))} />
                  </Field>
                )}
                {kind === "isosurface" && (
                  <NumberField label={t("export.potential")} unit={t("units.V")} value={iso} step={1} precision={3} onChange={setPotential} />
                )}
                {kind === "isosurface" && (
                  <p className={sim.hint}>{t("export.potentialRange", { min: num(o.info.potentialMin, 3), max: num(o.info.potentialMax, 3) })}</p>
                )}
                {(kind === "geometry" || kind === "isosurface") && surfaceControls}
                {kind === "grid" && (
                  <>
                    {cylindrical && (
                      <Field label={t("export.gridKind")} hint={t(revolve ? "export.gridKind.vtsHint" : "export.gridKind.vtiHint")}>
                        <SegmentedControl<string>
                          ariaLabel={t("export.gridKind")}
                          value={revolve ? "vts" : "vti"}
                          onChange={(v) => setRevolve(v === "vts")}
                          options={[
                            { value: "vti", label: t("export.gridKind.vti") },
                            { value: "vts", label: t("export.gridKind.vts") },
                          ]}
                        />
                      </Field>
                    )}
                    {cylindrical && revolve && surfaceControls}
                    <Field label={t("export.field")} hint={t("export.fieldHint")}>
                      <Switch checked={field} label={t("export.field")} onChange={setField} />
                    </Field>
                  </>
                )}
                {kind === "trajectories" && (
                  <>
                    <Field label={t("export.format")}>
                      <SegmentedControl<"vtp" | "csv">
                        ariaLabel={t("export.format")}
                        value={trajFormat}
                        onChange={setTrajFormat}
                        options={[
                          { value: "vtp", label: "VTP" },
                          { value: "csv", label: "CSV" },
                        ]}
                      />
                    </Field>
                    <p className={sim.hint}>{t("export.trajectories.count", { n: o.flight.result?.header.drawn ?? 0, ions: o.flight.result?.header.ions ?? 0 })}</p>
                  </>
                )}
                {kind === "pa" && (
                  <div className={sim.modes} role="radiogroup" aria-label={t("export.format")}>
                    {(["pa0", "pa", "paHash"] as const).map((f) => {
                      const off = (f === "pa0" && o.fast?.state !== "ready") || (f === "pa" && o.formatId === "paHash");
                      return (
                        <button key={f} type="button" role="radio" aria-checked={pa === f} className={`${sim.mode} ${styles.paMode}`} disabled={off} onClick={() => setPaFormat(f)}>
                          <Waves className={sim.modeIcon} aria-hidden />
                          <span className={sim.modeName}>{t(`export.pa.${f}`)}</span>
                          <span className={sim.modeSub}>{t(`export.pa.${f}Sub`)}</span>
                        </button>
                      );
                    })}
                    <p className={sim.modesHint}>{t(`export.pa.${pa}Hint`)}</p>
                  </div>
                )}
                {placement && kind !== "pa" && (
                  <p className={sim.hint}>{t(kind === "grid" && isRotated(placement) ? "export.placement.gridRotated" : "export.placement")}</p>
                )}
              </>
            )}
            {done && (
              <Notice tone="success" title={t("export.done", { n: done.files.length })} body={t("export.doneBody", { files: done.files.join(", "), dir: done.dir })}>
                {done.pa && (
                  <div className={sim.row}>
                    <Button size="sm" icon={<FolderOpen />} onClick={() => void reopen()}>
                      {t("export.reopen")}
                    </Button>
                  </div>
                )}
              </Notice>
            )}
            {error && <Notice tone="danger" title={t("export.failed")} body={tm(error)} />}
          </div>
        </div>
        <div className={styles.footer}>
          <span className={`${sim.hint} ${styles.footerHint}`}>{t("export.footer", { file: target().name })}</span>
          <Button variant="ghost" onClick={close} disabled={busy}>
            {t(done ? "export.closeDone" : "export.cancel")}
          </Button>
          <Button variant="primary" icon={done ? <Check /> : <Download />} onClick={() => void run()} disabled={busy || !!blocked}>
            {t(busy ? "export.working" : "export.run")}
          </Button>
        </div>
      </div>
    </div>
  );
}
