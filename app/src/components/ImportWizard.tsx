import { open } from "@tauri-apps/plugin-dialog";
import { Box, Check, CircleAlert, Cpu, FolderOpen, Grid3x3, Loader2, Play, Sparkles, Wand2, Wrench, X, Zap } from "lucide-react";
import { formatBytes, useI18n } from "../i18n";
import type { MessageKey } from "../i18n/en";
import type { BuildMethod, CadSurface, RefineDevice } from "../scene/backend";
import { gridCost, type MeshIssues } from "../scene/geometry";
import { closeWizard, reinspect, runWizard } from "../scene/importWizard";
import type { Vec3 } from "../scene/mesh";
import { cancelRefine, flyNow } from "../scene/simulation";
import { useApp } from "../state/store";
import { useWizard, wizardDims, type WizardPart, type WizardState, type WizardStep } from "../state/wizard";
import { Notice, RefineProgress } from "./SimulationSections";
import { Badge } from "./ui/Badge";
import { Button } from "./ui/Button";
import { Field, ReadOnlyValue } from "./ui/Field";
import { NumberField } from "./ui/NumberField";
import { Section } from "./ui/Section";
import { SegmentedControl } from "./ui/SegmentedControl";
import { Switch } from "./ui/Switch";
import { Vec3Field } from "./ui/Vec3Field";
import inputs from "./ui/inputs.module.css";
import panel from "./Panel.module.css";
import sim from "./SimulationSections.module.css";
import styles from "./ImportWizard.module.css";

const STEPS: readonly WizardStep[] = ["parts", "grid", "refine"];
/** Most grid points the wizard builds (src-tauri/src/geometry.rs MAX_POINTS). */
const MAX_POINTS = 40_000_000;
const LARGE_POINTS = 10_000_000;
const UNITS = ["mm", "cm", "m", "in"] as const;

const set = (patch: Partial<WizardState>) => useWizard.getState().set(patch);

/** Why the wizard cannot build yet (null = ready). */
function blocker(w: WizardState): { key: MessageKey; params?: Record<string, string | number> } | null {
  const info = w.info;
  if (!info) return { key: "wizard.blocked.reading" };
  if (info.gem && (!info.gem.refinable || !info.gem.electrostatic)) return { key: "wizard.blocked.gemKind" };
  const open = info.parts.find((p) => !p.closed);
  if (open) return { key: "wizard.blocked.open", params: { part: w.parts[open.index]?.name ?? open.name } };
  const bad = w.parts.find((p) => !Number.isInteger(p.number) || p.number < 1 || p.number > 1000);
  if (bad) return { key: "wizard.blocked.number", params: { part: bad.name } };
  const byNumber = new Map<number, number>();
  for (const p of w.parts) {
    const v = byNumber.get(p.number);
    if (v !== undefined && v !== p.voltage) return { key: "wizard.blocked.shared", params: { n: p.number } };
    byNumber.set(p.number, p.voltage);
  }
  const points = wizardDims(w).reduce((a, b) => a * b, 1);
  if (points > MAX_POINTS) return { key: "wizard.blocked.tooLarge", params: { points, limit: MAX_POINTS } };
  if (!w.outputDir || !w.stem.trim()) return { key: "wizard.blocked.output" };
  return null;
}

export function ImportWizard() {
  const { t, tm } = useI18n();
  const w = useWizard((s) => s.wizard);
  if (!w) return null;
  const busy = w.status === "building" || w.status === "refining" || w.status === "inspecting";
  const index = STEPS.indexOf(w.step);
  const block = blocker(w);
  const started = w.objectId !== null || w.status === "building";

  return (
    <aside className={`${panel.panel} ${styles.wizard}`} aria-label={t("wizard.title")}>
      <div className={panel.header}>
        <Wand2 className={styles.headerIcon} aria-hidden />
        <h2 className={panel.title}>{t("wizard.title")}</h2>
        <Button variant="ghost" size="sm" iconOnly icon={<X />} aria-label={t("wizard.close")} title={t("wizard.close")} onClick={closeWizard} />
      </div>
      <FileCard w={w} />
      <ol className={styles.steps}>
        {STEPS.map((step, i) => (
          <li key={step}>
            <button
              type="button"
              className={styles.step}
              data-active={step === w.step}
              data-done={i < index || w.status === "done"}
              disabled={started || !w.info}
              onClick={() => set({ step })}
            >
              <span className={styles.stepNumber}>{i < index || w.status === "done" ? <Check aria-hidden /> : i + 1}</span>
              <span className={styles.stepLabel}>{t(`wizard.step.${step}`)}</span>
            </button>
          </li>
        ))}
      </ol>
      <div className={panel.body}>
        {w.status === "error" && w.error && w.step !== "refine" && (
          <div className={styles.pad}>
            {w.cadUnavailable ? (
              <Notice tone="warning" title={t("wizard.cad.unavailable.title")} body={t("wizard.cad.unavailable.body")}>
                <p className={styles.detail}>{tm(w.error)}</p>
              </Notice>
            ) : w.error.key === "error.technical" ? (
              // Raw parser / OS text: say what to do first, keep the cause selectable underneath.
              <Notice tone="danger" title={t("wizard.failed")} body={t("wizard.failedNext")}>
                <p className={styles.detail}>{tm(w.error).replaceAll("\\\\?\\", "")}</p>
              </Notice>
            ) : (
              <Notice tone="danger" title={t("wizard.failed")} body={tm(w.error)} />
            )}
          </div>
        )}
        {w.step === "parts" && <PartsStep w={w} />}
        {w.step === "grid" && <GridStep w={w} />}
        {w.step === "refine" && <RefineStep w={w} />}
      </div>
      {/* While building/refining the footer has nothing to offer (cancel sits with the progress): give the room to the body. */}
      {!(started && w.status !== "done") && (
        <div className={styles.footer}>
          {block && w.status !== "done" && !started && index === STEPS.length - 1 && <p className={styles.blocker}>{t(block.key, block.params)}</p>}
          <div className={styles.footerRow}>
            {index > 0 && !started && (
              <Button variant="ghost" onClick={() => set({ step: STEPS[index - 1]! })} disabled={busy}>
                {t("wizard.back")}
              </Button>
            )}
            <span className={styles.spacer} />
            {w.status === "done" ? (
              <Button variant="primary" icon={<Check />} onClick={closeWizard}>
                {t("wizard.finish")}
              </Button>
            ) : index < STEPS.length - 1 ? (
              <Button variant="primary" onClick={() => set({ step: STEPS[index + 1]! })} disabled={!w.info || w.status === "inspecting"}>
                {t("wizard.next")}
              </Button>
            ) : (
              !started && (
                <Button variant="primary" icon={<Sparkles />} onClick={() => void runWizard()} disabled={!!block || busy}>
                  {t("wizard.build")}
                </Button>
              )
            )}
          </div>
        </div>
      )}
    </aside>
  );
}

function FileCard({ w }: { w: WizardState }) {
  const { t, num } = useI18n();
  const info = w.info;
  return (
    <div className={styles.file}>
      <span className={styles.fileIcon}>{w.status === "inspecting" ? <Loader2 className={styles.spin} /> : <Box />}</span>
      <div className={styles.fileText}>
        <p className={styles.fileName} title={w.path}>
          {w.fileName}
        </p>
        <p className={styles.fileMeta}>
          {info
            ? t(`wizard.file.${info.kind}`, { parts: info.parts.length, triangles: num(info.previewTriangles) })
            : w.status === "error"
              ? t("wizard.failed")
              : t("wizard.reading")}
        </p>
      </div>
      {info && <Badge tone={info.kind === "mesh" ? "info" : "accent"}>{info.kind === "gem" ? "GEM" : t(`wizard.badge.${info.kind}`)}</Badge>}
    </div>
  );
}

// -------------------------------------------------------------------------------- parts

function issueList(t: ReturnType<typeof useI18n>["t"], i: MeshIssues): string[] {
  const out: string[] = [];
  if (i.holes) out.push(t("wizard.issue.holes", { n: i.holes }));
  if (i.nonManifoldEdges) out.push(t("wizard.issue.nonManifold", { n: i.nonManifoldEdges }));
  if (i.flipped) out.push(t("wizard.issue.flipped", { n: i.flipped }));
  if (i.duplicateVertices) out.push(t("wizard.issue.dupVertices", { n: i.duplicateVertices }));
  if (i.degenerateTriangles + i.duplicateTriangles) out.push(t("wizard.issue.badTriangles", { n: i.degenerateTriangles + i.duplicateTriangles }));
  return out;
}

function PartsStep({ w }: { w: WizardState }) {
  const { t } = useI18n();
  const info = w.info;
  if (!info) return w.status === "error" ? null : <div className={styles.pad}><p className={sim.hint}>{t("wizard.reading")}</p></div>;
  const mesh = info.kind === "mesh";
  const cad = info.kind === "cad";
  const reports = info.parts.map((p) => p.report);
  const anyIssue = reports.some((r) => r && issueList(t, r.before).length > 0);
  const fixed = reports.reduce((n, r) => n + (r ? r.removedTriangles + r.flippedTriangles : 0), 0);
  const updatePart = (i: number, patch: Partial<WizardPart>) => set({ parts: w.parts.map((p, k) => (k === i ? { ...p, ...patch } : p)) });

  return (
    <>
      {(mesh || cad) && (
        <Section title={t("wizard.section.source")}>
          <Field label={t("wizard.units")} hint={t("wizard.units.hint", { unit: info.defaultUnits === "model" ? t("wizard.units.model") : info.defaultUnits })}>
            <SegmentedControl<string>
              ariaLabel={t("wizard.units")}
              value={w.units ?? "file"}
              onChange={(u) => void reinspect({ units: u === "file" ? null : u })}
              options={[{ value: "file", label: t("wizard.units.file"), title: t("wizard.units.fileTitle", { unit: info.defaultUnits === "model" ? t("wizard.units.model") : info.defaultUnits }) }, ...UNITS.map((u) => ({ value: u, label: u }))]}
            />
          </Field>
          {cad && (
            <div className={styles.report} data-tone="success">
              <Check className={styles.reportIcon} aria-hidden />
              <div>
                <p className={styles.reportTitle}>{t("wizard.report.ok", { n: info.parts.length })}</p>
                <p className={styles.reportBody}>{t("wizard.cad.report", { n: info.parts.length })}</p>
              </div>
            </div>
          )}
          {mesh && (
          <>
          <Field label={t("wizard.repair")} hint={t("wizard.repair.hint")}>
            <Switch checked={w.repair} label={t("wizard.repair")} onChange={(repair) => void reinspect({ repair })} />
          </Field>
          <div className={styles.report} data-tone={info.parts.every((p) => p.closed) ? "success" : "danger"}>
            {info.parts.every((p) => p.closed) ? <Check className={styles.reportIcon} aria-hidden /> : <CircleAlert className={styles.reportIcon} aria-hidden />}
            <div>
              <p className={styles.reportTitle}>
                {info.parts.every((p) => p.closed) ? t("wizard.report.ok", { n: info.parts.length }) : t("wizard.report.bad", { n: info.parts.filter((p) => !p.closed).length })}
              </p>
              <p className={styles.reportBody}>
                {!anyIssue ? t("wizard.report.clean") : w.repair ? t("wizard.report.repaired", { n: fixed }) : t("wizard.report.unrepaired")}
              </p>
            </div>
          </div>
          </>
          )}
        </Section>
      )}
      <Section title={t("wizard.section.parts", { n: info.parts.length })}>
        <p className={sim.lead}>{t(mesh ? "wizard.parts.leadMesh" : cad ? "wizard.parts.leadCad" : "wizard.parts.leadGem")}</p>
        <div className={styles.partsHead} aria-hidden>
          <span />
          <span>{t("wizard.parts.name")}</span>
          <span>{t("wizard.parts.number")}</span>
          <span>{t("wizard.parts.voltage")}</span>
        </div>
        <ul className={styles.parts}>
          {info.parts.map((p, i) => {
            const part = w.parts[i];
            if (!part) return null;
            const report = p.report;
            const before = report ? issueList(t, report.before) : [];
            const after = report?.after ? issueList(t, report.after) : null;
            return (
              <li
                key={p.index}
                className={styles.part}
                data-selected={w.highlight === i}
                data-open={!p.closed}
                onPointerEnter={() => set({ highlight: i })}
                onPointerLeave={() => useWizard.getState().wizard?.highlight === i && set({ highlight: null })}
              >
                <span className={styles.swatch} style={{ background: `var(--vp-part-${((part.number - 1) % 6) + 1})` }} aria-hidden />
                <input className={`${inputs.input} ${styles.nameInput}`} value={part.name} aria-label={t("wizard.parts.name")} onChange={(e) => updatePart(i, { name: e.target.value })} />
                <input
                  className={`${inputs.input} ${inputs.number} ${styles.numInput}`}
                  inputMode="numeric"
                  value={part.number}
                  aria-label={t("wizard.parts.number")}
                  onChange={(e) => updatePart(i, { number: Math.round(Number(e.target.value) || 0) })}
                />
                <span className={styles.voltWrap}>
                  <input
                    // Remount when the voltage changes elsewhere (the field is edited uncommitted until blur).
                    key={part.voltage}
                    className={`${inputs.input} ${inputs.number} ${styles.voltInput}`}
                    inputMode="decimal"
                    defaultValue={part.voltage}
                    aria-label={t("wizard.parts.voltage")}
                    onBlur={(e) => {
                      const v = Number(e.target.value.replace(",", "."));
                      if (Number.isFinite(v)) updatePart(i, { voltage: v });
                      else e.target.value = String(part.voltage);
                    }}
                    onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
                  />
                  <span className={styles.voltUnit}>V</span>
                </span>
                {p.exact !== null && (
                  <p className={styles.partIssues} data-tone={p.exact ? "ok" : "warn"} title={t(p.exact ? "wizard.cad.exactTitle" : "wizard.cad.invalidTitle")}>
                    {p.exact ? <Check aria-hidden /> : <CircleAlert aria-hidden />}
                    {t(p.exact ? "wizard.cad.exact" : "wizard.cad.invalid")}
                  </p>
                )}
                {(before.length > 0 || !p.closed) && (
                  <p className={styles.partIssues} data-tone={p.closed ? "fixed" : "open"}>
                    {!p.closed ? <CircleAlert aria-hidden /> : <Wrench aria-hidden />}
                    {after ? (after.length ? after.join(" · ") : t("wizard.issue.fixed", { list: before.join(" · ") })) : before.join(" · ")}
                  </p>
                )}
              </li>
            );
          })}
        </ul>
        {info.kind === "gem" && info.gem && (
          <p className={sim.hint}>
            {t("wizard.gem.grid", {
              dims: info.gem.dims.filter((n, a) => a < 2 || n > 1).join(" × "),
              spacing: info.gem.spacing[0],
              symmetry: t(info.gem.symmetry === "cylindrical" ? "wizard.gem.cylindrical" : "wizard.gem.planar"),
            })}
          </p>
        )}
      </Section>
    </>
  );
}

// --------------------------------------------------------------------------------- grid

function GridStep({ w }: { w: WizardState }) {
  const { t, num, lang } = useI18n();
  const info = w.info!;
  const gem = info.gem;
  const s = info.suggestion;
  const dims = wizardDims(w);
  const points = dims.reduce((a, b) => a * b, 1);
  const numbers = new Set(w.parts.map((p) => p.number)).size;
  const cost = gridCost(points, numbers);
  const locked = !!gem && w.method === "aligned";
  const methods: BuildMethod[] = ["cutcell", "aligned"];
  const reason =
    s.limitedBy === "gap" && s.gapMm !== null
      ? t("wizard.suggest.gap", { gap: num(s.gapMm, 3) })
      : s.limitedBy === "feature" && s.featureMm !== null
        ? t("wizard.suggest.feature", { feature: num(s.featureMm, 3) })
        : s.limitedBy === "budget"
          ? t("wizard.suggest.budget")
          : t("wizard.suggest.paDefine");
  const tone = points > MAX_POINTS ? "danger" : points > LARGE_POINTS ? "warning" : "ok";

  return (
    <>
      <Section title={t("wizard.section.method")}>
        <div className={sim.modes} role="radiogroup" aria-label={t("wizard.section.method")}>
          {methods.map((m) => (
            <button key={m} type="button" role="radio" aria-checked={w.method === m} className={sim.mode} onClick={() => set({ method: m, ...(gem && m === "aligned" ? { spacing: gem.spacing[0] } : {}) })}>
              {m === "cutcell" ? <Sparkles className={sim.modeIcon} aria-hidden /> : <Grid3x3 className={sim.modeIcon} aria-hidden />}
              <span className={sim.modeName}>{t(`wizard.method.${m}`)}</span>
              <span className={sim.modeSub}>{t(`wizard.method.${m}Sub`)}</span>
            </button>
          ))}
          <p className={sim.modesHint}>{t(`wizard.method.${w.method}Hint`)}</p>
        </div>
      </Section>
      {info.kind === "cad" && (
        <Section title={t("wizard.section.cadSurface")}>
          <div className={sim.modes} role="radiogroup" aria-label={t("wizard.section.cadSurface")}>
            {(["tessellated", "exact"] as const satisfies readonly CadSurface[]).map((c) => (
              <button key={c} type="button" role="radio" aria-checked={w.cadSurface === c} className={sim.mode} onClick={() => set({ cadSurface: c })}>
                {c === "tessellated" ? <Zap className={sim.modeIcon} aria-hidden /> : <Sparkles className={sim.modeIcon} aria-hidden />}
                <span className={sim.modeName}>{t(`wizard.cadSurface.${c}`)}</span>
                <span className={sim.modeSub}>{t(`wizard.cadSurface.${c}Sub`)}</span>
              </button>
            ))}
            <p className={sim.modesHint}>{t(`wizard.cadSurface.${w.cadSurface}Hint`)}</p>
          </div>
        </Section>
      )}
      <Section title={t("wizard.section.grid")}>
        {locked ? (
          <Field label={t("wizard.spacing")} hint={t("wizard.spacing.locked")}>
            <ReadOnlyValue mono>{num(w.spacing, 4)} mm</ReadOnlyValue>
          </Field>
        ) : (
          <NumberField label={t("wizard.spacing")} unit="mm" value={w.spacing} step={w.spacing / 10} precision={4} min={1e-6} scrubTitle={t("props.scrubHint")} onChange={(spacing) => spacing > 0 && set({ spacing })} />
        )}
        {!locked && (
          <div className={styles.suggest}>
            <p>
              {t("wizard.suggest", { spacing: num(s.spacing, 4) })} <span className={sim.subtle}>{reason}</span>
            </p>
            {w.spacing !== s.spacing && !gem && (
              <Button size="sm" variant="ghost" onClick={() => set({ spacing: s.spacing })}>
                {t("wizard.suggest.use")}
              </Button>
            )}
          </div>
        )}
        {!gem && (
          <>
            <Vec3Field label={t("wizard.domain.origin")} unit="mm" value={w.origin} step={w.spacing} onChange={(origin) => set({ origin })} />
            <Vec3Field label={t("wizard.domain.size")} unit="mm" value={w.size} step={w.spacing} onChange={(size: Vec3) => set({ size: size.map((v) => Math.max(v, w.spacing)) as Vec3 })} />
            <div className={sim.row}>
              <Button size="sm" variant="ghost" onClick={() => set({ origin: s.origin, size: [0, 1, 2].map((a) => (s.dims[a]! - 1) * s.spacing) as Vec3 })}>
                {t("wizard.domain.reset")}
              </Button>
            </div>
          </>
        )}
        <dl className={styles.cost} data-tone={tone}>
          <div>
            <dt>{t("wizard.cost.dims")}</dt>
            <dd className="num">{dims.filter((n, a) => a < 2 || n > 1).join(" × ")}</dd>
          </div>
          <div>
            <dt>{t("wizard.cost.points")}</dt>
            <dd className="num">{new Intl.NumberFormat(lang === "ko" ? "ko-KR" : "en-US", { notation: "compact" }).format(points)}</dd>
          </div>
          <div>
            <dt>{t("wizard.cost.ram")}</dt>
            <dd className="num">≈ {formatBytes(lang, cost.ramBytes)}</dd>
          </div>
          <div>
            <dt>{t("wizard.cost.disk")}</dt>
            <dd className="num">≈ {formatBytes(lang, cost.diskBytes)}</dd>
          </div>
        </dl>
        <p className={tone === "ok" ? sim.hint : sim.warnHint}>
          {tone === "danger" ? t("wizard.cost.tooLarge", { limit: MAX_POINTS }) : tone === "warning" ? t("wizard.cost.large") : t("wizard.cost.hint", { n: numbers })}
        </p>
      </Section>
    </>
  );
}

// ------------------------------------------------------------------------------- refine

function RefineStep({ w }: { w: WizardState }) {
  const { t, tm, num } = useI18n();
  const o = useApp((s) => s.objects.find((x) => x.id === w.objectId));
  const pa = o?.kind === "pa" ? o : null;
  const numbers = [...new Set(w.parts.map((p) => p.number))].sort((a, b) => a - b);
  const files = [`${w.stem}.pa#`, `${w.stem}.pa0`, ...numbers.map((n) => `${w.stem}.pa${n}`), ...(w.method === "cutcell" && w.sidecar ? [`${w.stem}.cuts.json`] : [])];
  const started = w.objectId !== null || w.status === "building";
  const chooseFolder = async () => {
    const dir = await open({ directory: true, defaultPath: w.outputDir || undefined, title: t("refine.folder.pick") });
    if (typeof dir === "string") set({ outputDir: dir });
  };

  return (
    <>
      <Section title={t("wizard.section.output")}>
        <Field label={t("refine.folder")}>
          <span className={sim.path} title={w.outputDir}>
            {w.outputDir || "…"}
          </span>
        </Field>
        <div className={sim.row}>
          <Button size="sm" icon={<FolderOpen />} onClick={() => void chooseFolder()} disabled={started}>
            {t("refine.folder.change")}
          </Button>
        </div>
        <Field label={t("wizard.stem")}>
          <input className={inputs.input} value={w.stem} disabled={started} aria-label={t("wizard.stem")} onChange={(e) => set({ stem: e.target.value })} />
        </Field>
        <p className={sim.hint}>{t("refine.writes", { files: files.join(", ") })}</p>
        {w.method === "cutcell" && (
          <Field label={t("wizard.sidecar")} hint={t("wizard.sidecar.hint")}>
            <Switch checked={w.sidecar} label={t("wizard.sidecar")} onChange={(sidecar) => !started && set({ sidecar })} />
          </Field>
        )}
        <Field label={t("refine.device")} hint={t(`refine.device.${w.device}Hint`)}>
          <SegmentedControl<RefineDevice>
            ariaLabel={t("refine.device")}
            value={w.device}
            onChange={(device) => !started && set({ device })}
            options={[
              { value: "auto", label: t("refine.device.auto") },
              { value: "gpu", label: "GPU", icon: <Zap /> },
              { value: "cpu", label: "CPU", icon: <Cpu /> },
            ]}
          />
        </Field>
      </Section>
      {(started || w.status === "error") && (
        <Section title={t("wizard.section.progress")}>
          {w.status === "building" && (
            <p className={styles.working}>
              <Loader2 className={styles.spin} aria-hidden /> {t(w.method === "cutcell" ? "wizard.building.cutcell" : "wizard.building.aligned")}
            </p>
          )}
          {w.build && (
            <p className={sim.hint}>
              {t("wizard.built", {
                files: w.build.files.join(", "),
                ms: num(w.build.ms, 0),
                cut: w.build.cutNodes !== null ? t("wizard.built.cut", { n: num(w.build.cutNodes) }) : "",
              })}
            </p>
          )}
          {pa?.refine && (pa.refine.status === "running" || pa.refine.status === "starting") && <RefineProgress r={pa.refine} onCancel={() => cancelRefine(pa.id)} />}
          {w.status === "done" && pa && (
            <Notice tone="success" title={t("wizard.done.title")} body={t("wizard.done.body", { file: pa.fileName })}>
              <div className={sim.row}>
                <Button size="sm" variant="primary" icon={<Play />} onClick={() => void flyNow(pa.id)} disabled={pa.flight.status === "running"}>
                  {t(pa.flight.result ? "ions.flyAgain" : "ions.fly")}
                </Button>
              </div>
            </Notice>
          )}
          {w.status === "error" && <Notice tone="danger" title={t("wizard.failed")} body={w.error ? tm(w.error) : ""} />}
        </Section>
      )}
    </>
  );
}
