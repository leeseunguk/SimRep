import { useEffect, useState } from "react";
import { Check, Download, FileText, Printer, X } from "lucide-react";
import { useI18n, type Message } from "../i18n";
import { collectReport, reportArray, runLabel } from "../report/collect";
import { renderReport } from "../report/html";
import { canPrintPdf, printReport } from "../report/print";
import { reportTranscript } from "../report/transcript";
import { exportStem, saveFile } from "../results/exporters";
import { useRuns } from "../state/runs";
import { useApp } from "../state/store";
import { useReportDialog } from "../state/reportDialog";
import { fmtDate } from "../report/format";
import { Notice } from "./SimulationSections";
import { Button } from "./ui/Button";
import { Field } from "./ui/Field";
import { SegmentedControl } from "./ui/SegmentedControl";
import { Switch } from "./ui/Switch";
import dialog from "./Dialog.module.css";
import sim from "./SimulationSections.module.css";
import styles from "./ReportDialog.module.css";

type Format = "html" | "pdf";

/** Export report: format, what to include and which stored runs to compare. */
export function ReportDialog() {
  const { t, tm, lang } = useI18n();
  const close = useReportDialog((s) => s.close);
  const runs = useRuns((s) => s.runs);
  const flown = useApp((s) => s.objects.some((o) => o.kind === "pa" && o.flight.result !== null));
  const pdf = canPrintPdf();
  const [format, setFormat] = useState<Format>("html");
  const [selected, setSelected] = useState<string[]>([]);
  const [include3d, setInclude3d] = useState(true);
  const transcript = reportTranscript();
  const [withTranscript, setWithTranscript] = useState(true);
  const [busy, setBusy] = useState(false);
  const [done, setDone] = useState<string | null>(null);
  const [error, setError] = useState<Message | null>(null);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && close();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [close]);

  const toggle = (id: string) => setSelected((s) => (s.includes(id) ? s.filter((x) => x !== id) : [...s, id]));

  const run = async () => {
    setBusy(true);
    setError(null);
    setDone(null);
    try {
      const model = await collectReport({ runIds: selected, include3d, transcript: withTranscript ? transcript : null });
      const html = renderReport(model);
      if (format === "pdf") {
        try {
          await printReport(html);
        } catch (err) {
          setError({ key: "report.dialog.pdfFailed", params: { detail: err instanceof Error ? err.message : String(err) } });
        }
        return;
      }
      const o = reportArray();
      const path = await saveFile(`${exportStem(o)}-report.html`, "html", html, o);
      if (path !== null) setDone(path);
    } catch (err) {
      setError({ key: "error.technical", params: { detail: err instanceof Error ? err.message : String(err) } });
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className={dialog.scrim} onPointerDown={(e) => e.target === e.currentTarget && close()}>
      <div className={`${dialog.dialog} ${styles.dialog}`} role="dialog" aria-modal="true" aria-labelledby="report-title" data-testid="report-dialog">
        <div className={dialog.header}>
          <FileText className={dialog.headerIcon} aria-hidden />
          <h2 id="report-title" className={dialog.title}>
            {t("report.dialog.title")}
          </h2>
          <button type="button" className={dialog.close} aria-label={t("report.dialog.close")} onClick={close}>
            <X />
          </button>
        </div>
        <p className={sim.lead}>{t("report.dialog.lead")}</p>
        <Field label={t("report.dialog.format")} hint={format === "pdf" ? t("report.dialog.pdfHint") : undefined}>
          <SegmentedControl<Format>
            ariaLabel={t("report.dialog.format")}
            value={format}
            onChange={setFormat}
            options={[
              { value: "html", label: t("report.dialog.html") },
              ...(pdf ? [{ value: "pdf" as const, label: t("report.dialog.pdf") }] : []),
            ]}
          />
        </Field>
        {!pdf && <Notice tone="info" title={t("report.dialog.pdfUnavailable")} />}
        {!flown && <Notice tone="info" title={t("report.dialog.noFlight")} />}
        <fieldset className={styles.group}>
          <legend className={styles.legend}>{t("report.dialog.include")}</legend>
          <label className={styles.check}>
            <Switch checked={include3d} label={t("report.dialog.include3d")} onChange={setInclude3d} />
            <span>{t("report.dialog.include3d")}</span>
          </label>
          <label className={styles.check}>
            <Switch checked={!!transcript && withTranscript} label={t("report.dialog.includeTranscript")} onChange={setWithTranscript} />
            <span>{t("report.dialog.includeTranscript")}</span>
            {!transcript && <span className={sim.hint}>{t("report.dialog.transcriptNone")}</span>}
          </label>
        </fieldset>
        <fieldset className={styles.group}>
          <legend className={styles.legend}>{t("report.dialog.runs")}</legend>
          {runs.length === 0 ? (
            <p className={sim.hint}>{t("report.dialog.runsNone")}</p>
          ) : (
            <>
              <p className={sim.hint}>{t("report.dialog.runsHint")}</p>
              <ul className={styles.runs}>
                {runs.map((r) => (
                  <li key={r.id}>
                    <label className={styles.run}>
                      <input type="checkbox" checked={selected.includes(r.id)} onChange={() => toggle(r.id)} />
                      <span className={styles.runName}>{runLabel(lang, r)}</span>
                      <span className={styles.runMeta}>
                        {r.array.name} · {fmtDate(lang, new Date(r.createdAt))}
                      </span>
                    </label>
                  </li>
                ))}
              </ul>
            </>
          )}
        </fieldset>
        {done !== null && <Notice tone="success" title={t("toast.exported.title")} body={done || undefined} />}
        {error && <Notice tone="danger" title={t("report.dialog.failed")} body={tm(error)} />}
        <div className={styles.footer}>
          <Button variant="ghost" onClick={close}>
            {t(done !== null ? "export.closeDone" : "export.cancel")}
          </Button>
          <Button
            variant="primary"
            icon={done !== null ? <Check /> : format === "pdf" ? <Printer /> : <Download />}
            onClick={() => void run()}
            disabled={busy}
            data-testid="report-run"
          >
            {t(busy ? "report.dialog.working" : format === "pdf" ? "report.dialog.print" : "report.dialog.run")}
          </Button>
        </div>
      </div>
    </div>
  );
}
