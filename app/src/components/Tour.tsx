import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties } from "react";
import { ArrowLeft, ArrowRight, Check, GraduationCap, MousePointerClick, X } from "lucide-react";
import { useI18n } from "../i18n";
import type { MessageKey } from "../i18n/en";
import { useResults } from "../results/store";
import { useApp } from "../state/store";
import { TOUR_ELECTRODE, TOUR_STEPS, tourArray, useTour, type TourStep } from "../state/tour";
import { Button } from "./ui/Button";
import styles from "./Tour.module.css";

/** Guided tour: the first-launch offer, then a coach card next to what each step needs. */
export function Tour() {
  const mode = useTour((s) => s.mode);
  const home = useApp((s) => s.home);
  // On the start screen the offer sits in its side column (StartScreen renders it inline).
  if (mode === "offer") return home ? null : <TourOffer />;
  if (mode === "running") return <TourCoach />;
  return null;
}

/** The first-launch invitation: docked bottom-right, or `inline` in the start screen. */
export function TourOffer({ inline = false }: { inline?: boolean }) {
  const { t } = useI18n();
  const { start, close } = useTour.getState();
  return (
    <aside className={styles.card} data-docked={!inline || undefined} data-inline={inline || undefined} role="dialog" aria-labelledby="tour-offer-title">
      <div className={styles.head}>
        <GraduationCap className={styles.headIcon} aria-hidden />
        <p className={styles.kicker}>{t("tour.offer.kicker")}</p>
        <button type="button" className={styles.close} onClick={close} aria-label={t("tour.offer.later")} title={t("tour.offer.later")}>
          <X aria-hidden />
        </button>
      </div>
      <h2 id="tour-offer-title" className={styles.title}>
        {t("tour.offer.title")}
      </h2>
      <p className={styles.body}>{t("tour.offer.body")}</p>
      <ol className={styles.outline}>
        {TOUR_STEPS.filter((s) => s !== "done").map((s) => (
          <li key={s}>{t(`tour.${s}.title` as MessageKey)}</li>
        ))}
      </ol>
      <div className={styles.actions}>
        <span className={styles.later}>{t("tour.offer.laterHint")}</span>
        <Button size="sm" variant="ghost" onClick={close}>
          {t("tour.offer.later")}
        </Button>
        <Button size="sm" variant="primary" icon={<ArrowRight />} onClick={start}>
          {t("tour.offer.start")}
        </Button>
      </div>
    </aside>
  );
}

/** Whether the scene has moved past the step (the step's action is done). */
function useStepDone(step: TourStep): boolean {
  const mark = useTour((s) => s.mark);
  const home = useApp((s) => s.home);
  const busy = useApp((s) => s.busy !== null);
  const o = useApp((s) => tourArray(s.objects));
  const exported = useApp((s) =>
    s.logs.some((l) => l.message.key === "log.exported" && l.time >= mark.time && /\.csv$/i.test(String(l.message.params?.path ?? ""))),
  );
  const histogram = useResults((s) => s.open && s.tab === "hist" && s.hist.quantity === "tof");
  switch (step) {
    case "template":
      return !!o?.flight.result && !home && !busy;
    case "voltage": {
      const v = o?.voltages[TOUR_ELECTRODE];
      return v !== undefined && v !== mark.voltage;
    }
    case "fly":
      return !!o?.flight.result && o.flight.status === "done" && o.flight.result !== mark.flight;
    case "histogram":
      return histogram;
    case "export":
      return exported;
    case "done":
      return true;
  }
}

/** The element a step points at (`data-tour` attribute), by what is on screen now. */
function useTargetName(step: TourStep): string | null {
  const home = useApp((s) => s.home);
  const onHistogram = useResults((s) => s.open && s.tab === "hist");
  switch (step) {
    case "template":
      return home ? "template-einzel" : "home";
    case "voltage":
      return `voltage-${TOUR_ELECTRODE}`;
    case "fly":
      return "fly";
    case "histogram":
      return onHistogram ? "results" : "results-tab-hist";
    case "export":
      return "results-export";
    case "done":
      return null;
  }
}

interface Box {
  left: number;
  top: number;
  width: number;
  height: number;
}

const sameBox = (a: Box | null, b: Box | null) =>
  a === b || (!!a && !!b && a.left === b.left && a.top === b.top && a.width === b.width && a.height === b.height);

/**
 * Centers `el` in its nearest scrolling panel. Unlike `scrollIntoView`, never scrolls the
 * `overflow: hidden` app root, which would shift the whole window.
 */
function scrollIntoPanel(el: HTMLElement) {
  for (let p = el.parentElement; p; p = p.parentElement) {
    if (!/(auto|scroll)/.test(getComputedStyle(p).overflowY) || p.scrollHeight <= p.clientHeight) continue;
    const r = el.getBoundingClientRect();
    const box = p.getBoundingClientRect();
    p.scrollTop += r.top + r.height / 2 - (box.top + box.height / 2);
    return;
  }
}

function cssPx(name: string): number {
  return parseFloat(getComputedStyle(document.documentElement).getPropertyValue(name));
}

/**
 * Card position next to `target` (right, left, below, above: the first that fits the window;
 * `above` first when the target opens a menu below itself), or docked bottom-right above the
 * status bar when there is no target.
 */
function placeCard(target: Box | null, card: { width: number; height: number }, aboveFirst = false): { left: number; top: number } {
  const gap = cssPx("--space-3");
  const margin = cssPx("--space-4");
  const vw = window.innerWidth;
  const vh = window.innerHeight - cssPx("--statusbar-h");
  const clampX = (x: number) => Math.min(Math.max(x, margin), vw - card.width - margin);
  const clampY = (y: number) => Math.min(Math.max(y, margin), vh - card.height - margin);
  if (!target) return { left: vw - card.width - margin, top: vh - card.height - margin };
  const right = target.left + target.width;
  const bottom = target.top + target.height;
  const options = [
    { left: right + gap, top: clampY(target.top), fits: right + gap + card.width <= vw - margin },
    { left: target.left - gap - card.width, top: clampY(target.top), fits: target.left - gap - card.width >= margin },
    { left: clampX(target.left), top: bottom + gap, fits: bottom + gap + card.height <= vh - margin },
    { left: clampX(target.left), top: target.top - gap - card.height, fits: target.top - gap - card.height >= margin },
  ];
  if (aboveFirst) options.unshift(options.pop()!);
  const pick = options.find((o) => o.fits);
  return pick ? { left: pick.left, top: pick.top } : { left: vw - card.width - margin, top: vh - card.height - margin };
}

function TourCoach() {
  const { t } = useI18n();
  const index = useTour((s) => s.step);
  const step = TOUR_STEPS[index]!;
  const { go, close } = useTour.getState();
  const done = useStepDone(step);
  const targetName = useTargetName(step);
  const busy = useApp((s) => s.busy !== null);
  const helpOpen = useApp((s) => s.helpOpen);
  const card = useRef<HTMLElement>(null);
  const [target, setTarget] = useState<Box | null>(null);
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null);
  const scrolled = useRef<{ key: string; tries: number }>({ key: "", tries: 0 });
  const steps = TOUR_STEPS.length - 1;

  // Steps whose action changes the screen completely move on by themselves.
  useEffect(() => {
    if (done && (step === "template" || step === "export")) go(index + 1);
  }, [done, step, index, go]);

  // Follow the target (panels resize, the results panel opens, lists scroll) every frame.
  useLayoutEffect(() => {
    let frame = 0;
    let last: Box | null = null;
    const tick = () => {
      const el = targetName ? document.querySelector<HTMLElement>(`[data-tour="${targetName}"]`) : null;
      const r = el?.getBoundingClientRect();
      const box = r && r.width > 0 && r.height > 0 ? { left: r.left, top: r.top, width: r.width, height: r.height } : null;
      // Bring the target on screen (panels scroll); retry a few times while layout settles.
      const key = `${index}:${targetName}`;
      if (scrolled.current.key !== key) scrolled.current = { key, tries: 0 };
      const hidden = box && (box.top < 0 || box.top + box.height > window.innerHeight - cssPx("--statusbar-h"));
      if (el && hidden && scrolled.current.tries < 5) {
        scrolled.current.tries++;
        scrollIntoPanel(el);
      }
      if (!sameBox(box, last)) {
        last = box;
        setTarget(box);
      }
      const c = card.current;
      if (c) {
        // The export button opens its menu below and to the left: keep the card out of its way.
        const next = placeCard(box, { width: c.offsetWidth, height: c.offsetHeight }, step === "export");
        setPos((p) => (p && p.left === next.left && p.top === next.top ? p : next));
      }
      frame = requestAnimationFrame(tick);
    };
    tick();
    return () => cancelAnimationFrame(frame);
  }, [targetName, index, step]);

  if (busy || helpOpen) return null;
  const n = Math.min(index + 1, steps);
  const final = step === "done";
  const todoKey = (step === "template" && targetName === "home" ? "tour.template.todoHome" : `tour.${step}.todo`) as MessageKey;

  return (
    <>
      {target && !final && (
        <div
          className={styles.ring}
          aria-hidden
          style={{ left: target.left, top: target.top, width: target.width, height: target.height } as CSSProperties}
        />
      )}
      <aside
        ref={card}
        className={styles.card}
        data-placed={pos !== null}
        style={pos ? ({ left: pos.left, top: pos.top } as CSSProperties) : undefined}
        role="dialog"
        aria-live="polite"
        aria-labelledby="tour-title"
      >
        <div className={styles.head}>
          <GraduationCap className={styles.headIcon} aria-hidden />
          <p className={styles.kicker}>{final ? t("tour.kickerDone") : t("tour.kicker", { n, total: steps })}</p>
          <button type="button" className={styles.close} onClick={close} aria-label={t("tour.skip")} title={t("tour.skip")}>
            <X aria-hidden />
          </button>
        </div>
        <div className={styles.progress} role="progressbar" aria-label={t("tour.progress")} aria-valuemin={0} aria-valuemax={steps} aria-valuenow={final ? steps : index}>
          {TOUR_STEPS.slice(0, steps).map((s, i) => (
            <span key={s} className={styles.segment} data-state={i < index || final ? "done" : i === index ? "current" : "todo"} />
          ))}
        </div>
        <h2 id="tour-title" className={styles.title}>
          {t(`tour.${step}.title` as MessageKey)}
        </h2>
        <p className={styles.body}>{t(`tour.${step}.body` as MessageKey)}</p>
        {!final && (
          <p className={styles.status} data-done={done}>
            {done ? <Check aria-hidden /> : <MousePointerClick aria-hidden />}
            {done ? t(`tour.${step}.done` as MessageKey) : t(todoKey)}
          </p>
        )}
        <div className={styles.actions}>
          {!final && (
            <Button size="sm" variant="ghost" onClick={close}>
              {t("tour.skip")}
            </Button>
          )}
          <span className={styles.spacer} />
          {index > 0 && (
            <Button size="sm" variant="ghost" icon={<ArrowLeft />} onClick={() => go(index - 1)}>
              {t("tour.back")}
            </Button>
          )}
          {final ? (
            <Button size="sm" variant="primary" icon={<Check />} onClick={close}>
              {t("tour.finish")}
            </Button>
          ) : (
            <Button size="sm" variant={done ? "primary" : "secondary"} icon={<ArrowRight />} onClick={() => go(index + 1)}>
              {t("tour.next")}
            </Button>
          )}
        </div>
      </aside>
    </>
  );
}
