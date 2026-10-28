import { translate, type Message, type MessageParams } from "../i18n";
import type { MessageKey } from "../i18n/en";
import { colorRamp, heatRaster, type ChartSpec, type Layer } from "../results/plot/chart";
import type { Lang } from "../state/store";
import { fmtCount, fmtDate, fmtMs, fmtPercent, fmtResolution, fmtSig, fmtTriple } from "./format";
import type { ArraySummary, ReportModel, RunColumn, SettingsSummary, TranscriptItem } from "./model";
import type { ResultSummary } from "./summary";
import { chartSvg, PRINT_THEME, svgDataUri } from "./svg";

/**
 * The report as one self-contained HTML document: inline CSS (screen + A4 print), figures as
 * data: URIs (SVG drawn from the data, the 3D snapshot as PNG), no scripts and no network
 * references. Every text from the project (names, paths, transcript) is HTML-escaped.
 */

export const esc = (s: string) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;").replace(/'/g, "&#39;");

const FIG_W = 640;
const FIG_H = 300;

const CSS = `
:root { color-scheme: light; --ink: #1d1f23; --muted: #5b6068; --rule: #d9dbe0; --soft: #f4f5f7; --accent: #c86f00; --warn: #9a5b00; --warn-bg: #fff6e5; }
* { box-sizing: border-box; }
html { background: #e9eaee; }
body { margin: 0; color: var(--ink); font: 10.5pt/1.45 'IBM Plex Sans', 'IBM Plex Sans KR', 'Malgun Gothic', 'Apple SD Gothic Neo', 'Noto Sans KR', Arial, sans-serif; }
main { max-width: 210mm; margin: 24px auto; padding: 16mm 15mm; background: #fff; box-shadow: 0 2px 12px rgb(0 0 0 / 0.12); }
h1 { margin: 0 0 4mm; font-size: 20pt; line-height: 1.2; letter-spacing: -0.01em; }
h2 { margin: 8mm 0 3mm; padding-bottom: 1.5mm; border-bottom: 1.5pt solid var(--accent); font-size: 13pt; break-after: avoid; page-break-after: avoid; }
h3 { margin: 5mm 0 2mm; font-size: 11pt; break-after: avoid; page-break-after: avoid; }
p { margin: 0 0 2mm; }
.meta { display: grid; grid-template-columns: max-content 1fr; gap: 1mm 6mm; margin: 0 0 4mm; }
.meta dt { color: var(--muted); }
.meta dd { margin: 0; overflow-wrap: anywhere; }
.toc { margin: 0 0 2mm; padding-left: 5mm; color: var(--muted); columns: 2; }
table { width: 100%; margin: 0 0 3mm; border-collapse: collapse; font-size: 9.5pt; break-inside: auto; }
tr { break-inside: avoid; page-break-inside: avoid; }
th, td { padding: 1.2mm 2mm; border-bottom: 0.5pt solid var(--rule); text-align: left; vertical-align: top; overflow-wrap: anywhere; }
thead th { background: var(--soft); font-weight: 600; }
th[scope="row"] { width: 42%; color: var(--muted); font-weight: 400; }
td.num, th.num { text-align: right; font-family: 'IBM Plex Mono', Consolas, monospace; font-variant-numeric: tabular-nums; white-space: nowrap; }
.note { color: var(--muted); font-size: 9pt; }
.empty { color: var(--muted); font-style: italic; }
figure { margin: 0 0 5mm; break-inside: avoid; page-break-inside: avoid; }
figure img { display: block; max-width: 100%; height: auto; border: 0.5pt solid var(--rule); }
figcaption { margin-top: 1.5mm; color: var(--muted); font-size: 9pt; }
.warnings { margin: 0; padding: 0; list-style: none; }
.warnings li { margin: 0 0 1.5mm; padding: 1.5mm 3mm; border-left: 2pt solid var(--warn); background: var(--warn-bg); color: #3d2a00; break-inside: avoid; }
.chat { margin: 0 0 2.5mm; padding: 2mm 3mm; border: 0.5pt solid var(--rule); border-radius: 2mm; break-inside: avoid; }
.chat .role { margin-bottom: 1mm; color: var(--muted); font-size: 8.5pt; font-weight: 600; text-transform: uppercase; letter-spacing: 0.04em; }
.chat pre { margin: 0; font: inherit; white-space: pre-wrap; overflow-wrap: anywhere; }
.chat[data-role="tool"] pre { font-family: 'IBM Plex Mono', Consolas, monospace; font-size: 8.5pt; }
footer { margin-top: 8mm; padding-top: 2mm; border-top: 0.5pt solid var(--rule); color: var(--muted); font-size: 8.5pt; }
@page { size: A4; margin: 15mm 14mm 16mm; }
@media print {
  html { background: #fff; }
  main { max-width: none; margin: 0; padding: 0; box-shadow: none; }
  h2 { margin-top: 6mm; }
  a { color: inherit; text-decoration: none; }
  .warnings li { -webkit-print-color-adjust: exact; print-color-adjust: exact; }
  thead th { -webkit-print-color-adjust: exact; print-color-adjust: exact; }
}
`;

type T = (key: MessageKey, params?: MessageParams) => string;

function translator(lang: Lang): { t: T; tm: (m: Message) => string } {
  return { t: (key, params) => translate(lang, key, params), tm: (m) => translate(lang, m.key, m.params) };
}

const row = (k: string, v: string, numeric = false) => `<tr><th scope="row">${esc(k)}</th><td${numeric ? ' class="num"' : ""}>${esc(v)}</td></tr>`;
const table = (rows: string[]) => `<table><tbody>${rows.join("")}</tbody></table>`;
const img = (src: string, alt: string) => `<img src="${src}" alt="${esc(alt)}">`;
const figure = (src: string, caption: string) => `<figure>${img(src, caption)}<figcaption>${esc(caption)}</figcaption></figure>`;

// ----------------------------------------------------------------------------- sections

function geometry(a: ArraySummary | null, lang: Lang, t: T): string {
  if (!a) return `<p class="empty">${esc(t("report.noArray"))}</p>`;
  const axes = ["x", "y", "z"] as const;
  const mirrored = axes.filter((k) => a.mirror[k]);
  const anti = axes.filter((k) => a.mirror[k] && a.antiMirror[k]);
  const mirror = !mirrored.length ? t("props.pa.mirrorNone") : anti.length ? t("props.pa.mirrorAnti", { axes: mirrored.join(", "), anti: anti.join(", ") }) : mirrored.join(", ");
  const electrodes = a.electrodes.length
    ? `<table><thead><tr><th>${esc(t("report.electrode.name"))}</th><th class="num">${esc(t("report.electrode.number"))}</th><th class="num">${esc(t("report.electrode.voltage"))}</th></tr></thead><tbody>${a.electrodes
        .map(
          (e) =>
            `<tr><td>${esc(e.name)}</td><td class="num">${e.number === null ? "–" : esc(String(e.number))}</td><td class="num">${e.voltage === null ? esc(t("report.electrode.noVoltage")) : esc(fmtSig(lang, e.voltage, 8))}</td></tr>`,
        )
        .join("")}</tbody></table>`
    : `<p class="empty">${esc(t("report.electrode.none"))}</p>`;
  return (
    table([
      row(t("report.array.name"), a.name),
      row(t("report.array.file"), a.path ?? a.fileName),
      row(t("report.array.format"), t(`format.${a.format}` as MessageKey)),
      row(t("report.array.kind"), t(`pa.kind.${a.kind}` as MessageKey)),
      row(t("report.array.dims"), fmtTriple(lang, a.dims, fmtCount), true),
      row(t("report.array.points"), fmtCount(lang, a.points), true),
      row(t("report.array.spacing"), fmtTriple(lang, a.spacing), true),
      row(t("report.array.size"), fmtTriple(lang, a.sizeMm), true),
      row(t("report.array.mirror"), mirror),
      row(t("report.array.field"), t(`props.pa.field.${a.field}`)),
      row(t("report.array.electrodePoints"), `${fmtCount(lang, a.electrodePoints)} (${fmtPercent(lang, a.electrodePoints, a.points)})`, true),
      row(t("report.array.cuts"), a.cuts ?? t("report.array.cutsNone")),
    ]) + `<h3>${esc(t("report.electrodes"))}</h3>${electrodes}`
  );
}

function source(m: ReportModel, lang: Lang, t: T): string {
  const s = m.source;
  if (!s) return `<p class="empty">${esc(t("report.noArray"))}</p>`;
  const f = (v: number) => fmtSig(lang, v, 6);
  if (s.mode === "file")
    return table([row(t("ions.source"), t("report.source.file")), row(t("ions.file"), s.file?.path ?? t("ions.file.none"))]);
  return table([
    row(t("ions.source"), t("report.source.beam")),
    row(t("ions.count"), fmtCount(lang, s.count), true),
    row(`${t("ions.mass")}, amu`, f(s.massAmu), true),
    row(`${t("ions.charge")}, e`, f(s.chargeE), true),
    row(`${t("ions.energy")}, eV`, f(s.energyEv), true),
    row(`${t("ions.position")}, mm`, s.position.map(f).join(", "), true),
    row(`${t("report.source.direction")}, °`, `${f(s.azimuthDeg)}, ${f(s.elevationDeg)}`, true),
    row(`${t("ions.cone")}, °`, f(s.coneDeg), true),
    row(t("ions.shape"), t(`ions.shape.${s.shape}`)),
    ...(s.shape !== "point" ? [row(`${t(s.shape === "line" ? "ions.width.line" : "ions.width.disk")}, mm`, f(s.widthMm), true)] : []),
    row(t("ions.bench"), s.bench.enabled ? `${s.bench.min.map(f).join(", ")} → ${s.bench.max.map(f).join(", ")} mm` : t("report.no")),
  ]);
}

function settings(s: SettingsSummary | null, lang: Lang, t: T): string {
  if (!s) return `<p class="empty">${esc(t("report.noArray"))}</p>`;
  const yes = (b: boolean) => t(b ? "report.yes" : "report.no");
  const e = s.executed;
  const tol = s.tolerances;
  const refine = s.refine;
  return table([
    row(t("report.refine.device"), refine?.device ?? t("report.refine.none")),
    row(t("report.refine.tolerance"), fmtSig(lang, refine?.tolerance ?? 1e-10, 3), true),
    ...(refine?.cycles !== null && refine?.cycles !== undefined
      ? [row(t("report.refine.result"), t("report.refine.resultValue", { cycles: refine.cycles, residual: fmtSig(lang, refine.residual, 3) }))]
      : []),
    row(t("report.fly.requested"), t(s.precision === "live" ? "ions.precision.live" : "ions.precision.precise")),
    row(t("report.fly.device"), e ? `${e.device.toUpperCase()}${s.deviceName ? ` · ${s.deviceName}` : ""}` : t("report.fly.notFlown")),
    row(t("report.fly.precision"), e ? e.precision : "–"),
    row(t("report.fly.surfaceMode"), e ? t(e.surfaces === "exact" ? "results.surfaces.exact" : "results.surfaces.grid") : "–"),
    row(t("report.fly.surfacesRequested"), yes(s.exactSurfaces)),
    row(t("report.fly.collision"), t(`ions.collision.${s.collision}`)),
    row(t("report.fly.relativistic"), yes(s.relativistic)),
    row(t("report.fly.duration"), fmtSig(lang, s.durationUs), true),
    row(t("report.fly.drawn"), fmtCount(lang, s.drawn), true),
    row(
      t("report.fly.tolerances"),
      t("report.fly.tolerancesValue", {
        rel: fmtSig(lang, tol.relativeTolerance, 3),
        pos: fmtSig(lang, tol.positionToleranceMm, 3),
        vel: fmtSig(lang, tol.velocityToleranceMmPerUs, 3),
        step0: fmtSig(lang, tol.initialStepUs, 3),
        stepMax: fmtSig(lang, tol.maxStepUs, 3),
      }),
    ),
  ]);
}

/** Metric rows of one result (also the columns of the run comparison). */
export function metricRows(r: ResultSummary, lang: Lang, tm: (m: Message) => string, t: T): [string, string][] {
  const tof = (v: number | undefined) => fmtSig(lang, v ?? null, 7);
  return [
    [t("report.metric.ions"), fmtCount(lang, r.ions)],
    [t("report.metric.arrivals"), `${fmtCount(lang, r.arrivals)} (${fmtPercent(lang, r.arrivals, r.ions)})`],
    ...r.splats.map((s): [string, string] => [`${t("report.metric.splats")}: ${tm(s.label)}`, `${fmtCount(lang, s.count)} (${fmtPercent(lang, s.count, r.ions)})`]),
    [t("report.metric.tofMean"), tof(r.tof?.mean)],
    [t("report.metric.tofStd"), tof(r.tof?.std)],
    [t("report.metric.tofMin"), tof(r.tof?.min)],
    [t("report.metric.tofMax"), tof(r.tof?.max)],
    [t("report.metric.resolution"), r.resolution ? fmtResolution(lang, r.resolution.value) : "–"],
    [t("report.metric.keMean"), fmtSig(lang, r.keMean, 7)],
    [t("report.metric.compute"), fmtMs(lang, r.computeMs)],
  ];
}

function metrics(r: ResultSummary | null, lang: Lang, t: T, tm: (m: Message) => string): string {
  if (!r) return `<p class="empty">${esc(t("report.metric.none"))}</p>`;
  const head = `<thead><tr><th>${esc(t("report.metric.name"))}</th><th class="num">${esc(t("report.metric.value"))}</th></tr></thead>`;
  const body = metricRows(r, lang, tm, t)
    .map(([k, v]) => `<tr><td>${esc(k)}</td><td class="num">${esc(v)}</td></tr>`)
    .join("");
  const notes = [
    r.origin.kind === "run" ? t("report.metric.fromRun", { label: r.origin.label }) : null,
    r.resolution ? `${t("report.metric.resolution")}: ${t("report.metric.resolutionNote", { bins: r.resolution.bins, fwhm: fmtSig(lang, r.resolution.fwhm, 4) })}` : null,
    `compute_ms: ${t("report.metric.computeNote")}`,
  ].filter((x): x is string => x !== null);
  return `<table>${head}<tbody>${body}</tbody></table>${notes.map((n) => `<p class="note">${esc(n)}</p>`).join("")}`;
}

// ------------------------------------------------------------------------------- figures

function histSpec(h: NonNullable<ResultSummary["tofHist"]>, xLabel: string, yLabel: string, extra: Layer[] = []): ChartSpec {
  let max = 0;
  for (const c of h.counts) max = Math.max(max, c);
  return {
    x: { label: xLabel, domain: [h.lo, h.hi] },
    y: { label: yLabel, domain: [0, Math.max(1, max) * 1.08] },
    layers: [...extra.filter((l) => l.kind === "band"), { kind: "hist", lo: h.lo, hi: h.hi, counts: h.counts, fill: PRINT_THEME.primaryFill, stroke: PRINT_THEME.primary }, ...extra.filter((l) => l.kind !== "band")],
  };
}

/** The TOF histogram, energy histogram and detector map of a result, as SVG data URIs. */
export function resultFigures(r: ResultSummary, t: T, tm: (m: Message) => string): string {
  const out: string[] = [];
  const ions = t("results.axis.ions");
  if (r.tofHist) {
    const extra: Layer[] = [];
    if (r.resolution) extra.push({ kind: "band", x0: r.resolution.left, x1: r.resolution.right, color: PRINT_THEME.band });
    if (r.tof) extra.push({ kind: "vline", x: r.tof.mean, color: PRINT_THEME.textStrong, dash: [4, 3] });
    out.push(figure(svgDataUri(chartSvg(histSpec(r.tofHist, `${t("results.q.tof")} (µs)`, ions, extra), FIG_W, FIG_H)), t("report.fig.tof")));
  } else out.push(`<p class="empty">${esc(t("report.fig.tof"))}: ${esc(t("report.fig.noData"))}</p>`);
  if (r.keHist) out.push(figure(svgDataUri(chartSvg(histSpec(r.keHist, `${t("results.q.ke")} (eV)`, ions), FIG_W, FIG_H)), t("report.fig.ke")));
  if (r.map) {
    const m = r.map;
    const raster = heatRaster(m.counts, m.nx, m.ny, colorRamp(PRINT_THEME.heat), true);
    const spec: ChartSpec = {
      x: { label: `${m.axes[0]} (mm)`, domain: m.x },
      y: { label: `${m.axes[1]} (mm)`, domain: m.y },
      layers: [{ kind: "raster", raster, x: m.x, y: m.y }],
      equal: true,
    };
    const size = Math.min(FIG_W, 460);
    out.push(figure(svgDataUri(chartSvg(spec, size, size)), t("report.fig.map", { surface: tm(m.surface), plane: `${m.axes[0]}–${m.axes[1]}` })));
  }
  return out.join("");
}

function view3d(m: ReportModel, t: T, tm: (msg: Message) => string): string {
  if (!m.view3d) return "";
  if ("dataUri" in m.view3d) return figure(m.view3d.dataUri, t("report.fig.view3d"));
  return `<p class="note">${esc(t("report.view3d.unavailable", { reason: tm(m.view3d.unavailable) }))}</p>`;
}

// ----------------------------------------------------------------------------------- runs

/** Setting keys whose values differ between any of the runs (sorted). */
export function differingSettings(runs: RunColumn[]): string[] {
  const keys = new Set(runs.flatMap((r) => Object.keys(r.settings)));
  return [...keys].filter((k) => new Set(runs.map((r) => r.settings[k] ?? "–")).size > 1).sort();
}

function runs(m: ReportModel, lang: Lang, t: T, tm: (msg: Message) => string): string {
  const list = m.runs;
  const head = `<thead><tr><th>${esc(t("report.metric.name"))}</th>${list.map((r) => `<th class="num">${esc(r.label)}</th>`).join("")}</tr></thead>`;
  const rowsByRun = list.map((r) => metricRows(r.summary, lang, tm, t));
  // Union of metric names in order (end-state rows differ between runs).
  const names: string[] = [];
  for (const rows of rowsByRun) for (const [k] of rows) if (!names.includes(k)) names.push(k);
  const body = [
    `<tr><td>${esc(t("report.runs.created"))}</td>${list.map((r) => `<td class="num">${esc(fmtDate(lang, new Date(r.createdAt)))}</td>`).join("")}</tr>`,
    ...names.map((k) => `<tr><td>${esc(k)}</td>${rowsByRun.map((rows) => `<td class="num">${esc(rows.find(([n]) => n === k)?.[1] ?? "–")}</td>`).join("")}</tr>`),
  ].join("");
  let out = `<table>${head}<tbody>${body}</tbody></table>`;

  // Overlaid TOF histograms (outlines in series colours).
  const hists = list.map((r, i) => ({ h: r.summary.tofHist, color: PRINT_THEME.series[i % PRINT_THEME.series.length]!, label: r.label })).filter((x) => x.h);
  if (hists.length) {
    let lo = Infinity;
    let hi = -Infinity;
    let max = 0;
    for (const { h } of hists) {
      lo = Math.min(lo, h!.lo);
      hi = Math.max(hi, h!.hi);
      for (const c of h!.counts) max = Math.max(max, c);
    }
    const spec: ChartSpec = {
      x: { label: `${t("results.q.tof")} (µs)`, domain: [lo, hi] },
      y: { label: t("results.axis.ions"), domain: [0, Math.max(1, max) * 1.08] },
      layers: hists.map(({ h, color }) => ({ kind: "hist", lo: h!.lo, hi: h!.hi, counts: h!.counts, stroke: color }) as Layer),
      legend: hists.map(({ color, label }) => ({ label, color })),
    };
    out += figure(svgDataUri(chartSvg(spec, FIG_W, FIG_H)), t("report.fig.runsTof"));
  }

  out += `<h3>${esc(t("report.runs.settingsDiff"))}</h3>`;
  if (list.length < 2) return out + `<p class="note">${esc(t("report.runs.single"))}</p>`;
  const diff = differingSettings(list);
  if (!diff.length) return out + `<p class="note">${esc(t("report.runs.noDiff"))}</p>`;
  const dhead = `<thead><tr><th>${esc(t("report.runs.setting"))}</th>${list.map((r) => `<th>${esc(r.label)}</th>`).join("")}</tr></thead>`;
  const dbody = diff.map((k) => `<tr><td><code>${esc(k)}</code></td>${list.map((r) => `<td>${esc(r.settings[k] ?? "–")}</td>`).join("")}</tr>`).join("");
  return out + `<table>${dhead}<tbody>${dbody}</tbody></table>`;
}

function transcript(items: TranscriptItem[], t: T): string {
  const roleName = (role: string) => {
    const key = `report.transcript.role.${role}` as MessageKey;
    return ["user", "assistant", "tool", "system"].includes(role) ? t(key) : role;
  };
  return items
    .map((i) => {
      const role = i.tool ? `${roleName(i.role)} · ${i.tool}` : roleName(i.role);
      return `<div class="chat" data-role="${esc(i.tool ? "tool" : i.role)}"><div class="role">${esc(role)}</div><pre>${esc(i.text)}</pre></div>`;
    })
    .join("");
}

// ---------------------------------------------------------------------------------- page

export function renderReport(m: ReportModel): string {
  const { lang } = m;
  const { t, tm } = translator(lang);
  const sections: { id: string; title: MessageKey; body: string }[] = [
    { id: "geometry", title: "report.section.geometry", body: geometry(m.array, lang, t) },
    { id: "source", title: "report.section.source", body: source(m, lang, t) },
    { id: "settings", title: "report.section.settings", body: settings(m.settings, lang, t) },
    { id: "metrics", title: "report.section.metrics", body: metrics(m.results, lang, t, tm) },
    { id: "figures", title: "report.section.figures", body: (m.results ? resultFigures(m.results, t, tm) : `<p class="empty">${esc(t("report.metric.none"))}</p>`) + view3d(m, t, tm) },
    ...(m.runs.length ? [{ id: "runs", title: "report.section.runs" as const, body: runs(m, lang, t, tm) }] : []),
    {
      id: "warnings",
      title: "report.section.warnings",
      body: m.warnings.length ? `<ul class="warnings">${m.warnings.map((w) => `<li>${esc(tm(w))}</li>`).join("")}</ul>` : `<p class="note">${esc(t("report.warnings.none"))}</p>`,
    },
    ...(m.transcript?.length ? [{ id: "transcript", title: "report.section.transcript" as const, body: transcript(m.transcript, t) }] : []),
  ];
  const title = `${t("report.title")}: ${m.projectName}`;
  const meta = [
    [t("report.project"), m.projectName],
    [t("report.projectFile"), m.projectPath ?? t("report.projectUnsaved")],
    [t("report.generated"), fmtDate(lang, m.generatedAt)],
    [t("report.version"), m.simrepVersion],
  ];
  return `<!DOCTYPE html>
<html lang="${lang}">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="generator" content="SimRep ${esc(m.simrepVersion)}">
<title>${esc(title)}</title>
<style>${CSS}</style>
</head>
<body>
<main>
<h1>${esc(t("report.title"))}</h1>
<dl class="meta">${meta.map(([k, v]) => `<dt>${esc(k!)}</dt><dd>${esc(v!)}</dd>`).join("")}</dl>
<ol class="toc" aria-label="${esc(t("report.contents"))}">${sections.map((s) => `<li>${esc(t(s.title))}</li>`).join("")}</ol>
${sections.map((s) => `<section id="${s.id}"><h2>${esc(t(s.title))}</h2>${s.body}</section>`).join("\n")}
<footer>${esc(t("report.footer", { version: m.simrepVersion }))}</footer>
</main>
</body>
</html>
`;
}
