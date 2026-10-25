import { useEffect, useState } from "react";
import { ArrowUpLeft, Box, Boxes, Clock, Crosshair, FileQuestion, Info, Loader2, MousePointerClick, Trash2, Zap } from "lucide-react";
import { useI18n } from "../i18n";
import type { MessageKey } from "../i18n/en";
import { formatById } from "../scene/formats";
import { boundsCenter, boundsSize, scaleBounds } from "../scene/mesh";
import {
  coordRange,
  contourStep,
  displayMirrors,
  paKind,
  sliceAxes,
  spacing,
  type Axis,
  type ContourDensity,
} from "../scene/pa";
import { electrodeName, paElectrodes, type PaElectrode } from "../scene/electrodes";
import { objectName } from "../scene/names";
import { simulationAvailable } from "../scene/simulation";
import { SimulationSections, VoltageControl } from "./SimulationSections";
import {
  sceneTriangles,
  UNIT_SCALE,
  useApp,
  type MeshObject,
  type PaObject,
  type PendingObject,
  type SceneObject,
  type Units,
} from "../state/store";
import { Button } from "./ui/Button";
import { Field, ReadOnlyValue } from "./ui/Field";
import { NumberField } from "./ui/NumberField";
import { Section } from "./ui/Section";
import { SegmentedControl } from "./ui/SegmentedControl";
import { Switch } from "./ui/Switch";
import inputs from "./ui/inputs.module.css";
import panel from "./Panel.module.css";
import styles from "./PropertiesPanel.module.css";

export function PropertiesPanel() {
  const { t } = useI18n();
  const selected = useApp((s) => s.objects.find((o) => o.id === s.selectedId) ?? null);
  const label = useApp((s) => s.selectedElectrode);
  const electrode = selected?.kind === "pa" && label !== null ? (paElectrodes(selected).find((e) => e.label === label) ?? null) : null;

  return (
    <aside className={panel.panel}>
      <div className={panel.header}>
        <h2 className={panel.title}>{t("props.title")}</h2>
      </div>
      {/* Keyed by selection so a newly selected object starts scrolled to the top. */}
      <div className={panel.body} key={`${selected?.id ?? "none"}#${electrode?.label ?? ""}`}>
        {selected?.kind === "pa" && electrode ? (
          <ElectrodeProperties o={selected} e={electrode} />
        ) : selected ? (
          <ObjectProperties key={selected.id} o={selected} />
        ) : (
          <NothingSelected />
        )}
      </div>
    </aside>
  );
}

function NothingSelected() {
  const { t, num } = useI18n();
  const objects = useApp((s) => s.objects);
  const triangles = sceneTriangles(objects, false);
  return (
    <>
      <div className={styles.empty}>
        <MousePointerClick className={styles.emptyIcon} aria-hidden />
        <p className={styles.emptyTitle}>{t("props.none.title")}</p>
        <p className={styles.emptyBody}>{t("props.none.body")}</p>
      </div>
      <Section title={t("props.summary")}>
        <Field label={t("props.objects")}>
          <ReadOnlyValue mono>{num(objects.length)}</ReadOnlyValue>
        </Field>
        <Field label={t("props.triangles")}>
          <ReadOnlyValue mono>{num(triangles)}</ReadOnlyValue>
        </Field>
        <Field label={t("props.units")}>
          <ReadOnlyValue>mm</ReadOnlyValue>
        </Field>
      </Section>
    </>
  );
}

const TYPE_KEY: Record<SceneObject["kind"], MessageKey> = {
  mesh: "props.type.mesh",
  loading: "props.type.mesh",
  pending: "props.type.pending",
  pa: "props.type.pa",
};

function ObjectProperties({ o }: { o: SceneObject }) {
  const { t } = useI18n();
  const icon =
    o.kind === "mesh" ? (
      <Box />
    ) : o.kind === "pa" ? (
      <Boxes />
    ) : o.kind === "loading" ? (
      <Loader2 />
    ) : (
      <FileQuestion />
    );
  return (
    <>
      <div className={styles.identity}>
        <span className={styles.identityIcon}>{icon}</span>
        <div className={styles.identityText}>
          <p className={styles.identityName}>{objectName(o)}</p>
          <p className={styles.identityType}>{t(TYPE_KEY[o.kind])}</p>
        </div>
      </div>
      <Section title={t("props.section.general")}>
        <NameField o={o} />
        {(o.kind === "mesh" || o.kind === "pa") && (
          <Field label={t("props.visible")}>
            <Switch checked={o.visible} label={t("props.visible")} onChange={() => useApp.getState().toggleVisible(o.id)} />
          </Field>
        )}
      </Section>
      {o.kind === "mesh" && <MeshSections o={o} />}
      {o.kind === "pending" && <PendingSections o={o} />}
      {o.kind === "pa" && <PaSections o={o} />}
    </>
  );
}

function NameField({ o }: { o: SceneObject }) {
  const { t } = useI18n();
  const name = objectName(o);
  const [text, setText] = useState(name);
  useEffect(() => setText(name), [name]);
  return (
    <Field label={t("props.name")} htmlFor="prop-name">
      <input
        id="prop-name"
        className={inputs.input}
        value={text}
        onChange={(e) => setText(e.target.value)}
        onBlur={() => useApp.getState().rename(o.id, text === name ? (o.customName ?? "") : text)}
        onKeyDown={(e) => {
          if (e.key === "Enter") e.currentTarget.blur();
          if (e.key === "Escape") setText(name);
        }}
      />
    </Field>
  );
}

function MeshSections({ o }: { o: MeshObject }) {
  const { t, num, bytes } = useI18n();
  const b = scaleBounds(o.mesh.bounds, UNIT_SCALE[o.units]);
  const size = boundsSize(b);
  const center = boundsCenter(b);
  const vec = (v: number[]) => v.map((x) => num(Number(x.toFixed(2)))).join(" × ");
  const units: Units[] = ["mm", "cm", "m", "in"];
  return (
    <>
      <Section title={t("props.section.file")}>
        <Field label={t("props.fileName")}>
          <ReadOnlyValue>{o.fileName}</ReadOnlyValue>
        </Field>
        <Field label={t("props.fileSize")}>
          <ReadOnlyValue mono>{bytes(o.fileSize)}</ReadOnlyValue>
        </Field>
        <Field label={t("props.format")}>
          <ReadOnlyValue>{t(o.stlFormat === "binary" ? "format.stlBinary" : "format.stlAscii")}</ReadOnlyValue>
        </Field>
        <Field label={t("props.importUnits")} htmlFor="prop-units" hint={t("props.importUnitsHint")}>
          <select
            id="prop-units"
            className={`${inputs.input} ${inputs.select}`}
            value={o.units}
            onChange={(e) => useApp.getState().setUnits(o.id, e.target.value as Units)}
          >
            {units.map((u) => (
              <option key={u} value={u}>
                {t(`units.${u}`)}
              </option>
            ))}
          </select>
        </Field>
      </Section>
      <Section title={t("props.section.mesh")}>
        <Field label={t("props.triangles")}>
          <ReadOnlyValue mono>{num(o.mesh.triangleCount)}</ReadOnlyValue>
        </Field>
        <Field label={t("props.extent")}>
          <ReadOnlyValue mono>{vec(size)} mm</ReadOnlyValue>
        </Field>
        <Field label={t("props.center")}>
          <ReadOnlyValue mono>{vec(center)}</ReadOnlyValue>
        </Field>
        <ObjectActions o={o} focus />
      </Section>
    </>
  );
}

function PendingSections({ o }: { o: PendingObject }) {
  const { t, bytes } = useI18n();
  const format = formatById(o.formatId);
  return (
    <Section title={t("props.section.file")}>
      <Field label={t("props.fileName")}>
        <ReadOnlyValue>{o.fileName}</ReadOnlyValue>
      </Field>
      <Field label={t("props.fileSize")}>
        <ReadOnlyValue mono>{bytes(o.fileSize)}</ReadOnlyValue>
      </Field>
      <Field label={t("props.format")}>
        <ReadOnlyValue>{t(format.labelKey)}</ReadOnlyValue>
      </Field>
      <div className={styles.notice}>
        <Clock className={styles.noticeIcon} aria-hidden />
        <div>
          <p className={styles.noticeTitle}>{t("props.pending.title")}</p>
          <p className={styles.noticeBody}>{t("props.pending.body", { format: t(format.labelKey) })}</p>
        </div>
      </div>
      <ObjectActions o={o} />
    </Section>
  );
}

const AXIS_NAMES = ["X", "Y", "Z"] as const;
const PLANE_NAMES = ["YZ", "XZ", "XY"] as const;
const LIST_MAX = 8;

function shortList(values: readonly number[], fmt: (v: number) => string): string {
  const shown = values.slice(0, LIST_MAX).map(fmt).join(", ");
  return values.length > LIST_MAX ? `${shown}, …` : shown;
}

function PaSections({ o }: { o: PaObject }) {
  const { t, num, bytes } = useI18n();
  const { info } = o;
  const kind = paKind(info);
  const is3d = kind === "3d";
  const format = formatById(o.formatId);
  const d = spacing(info);
  const join = (values: number[]) => values.map((v) => num(v)).join(" × ");
  const shown = is3d ? 3 : 2;
  const cell = d.slice(0, shown).every((v) => v === d[0]) ? `${num(d[0])} mm` : `${join(d.slice(0, shown))} mm`;
  const extent = `${join([(info.nx - 1) * info.dx, (info.ny - 1) * info.dy, (info.nz - 1) * info.dz].slice(0, shown))} mm`;

  const flags = [info.mirror.x, info.mirror.y, info.mirror.z];
  const anti = [info.antiMirror.x, info.antiMirror.y, info.antiMirror.z];
  const mirrored = AXIS_NAMES.filter((_, i) => flags[i]).join(", ");
  const antiAxes = AXIS_NAMES.filter((_, i) => flags[i] && anti[i]).join(", ");
  const mirrorText = !mirrored
    ? t("props.pa.mirrorNone")
    : antiAxes
      ? t("props.pa.mirrorAnti", { axes: mirrored, anti: antiAxes })
      : mirrored;

  const electrodes =
    info.pa0 !== null
      ? t("props.pa.adjustable", { count: info.pa0.adjustable, list: shortList(info.pa0.electrodeNumbers, (v) => `#${v}`) })
      : info.distinctElectrodeValues === 0
        ? t("props.pa.noElectrodes")
        : o.formatId === "paHash"
          ? t("props.pa.electrodeNumbers", { count: info.distinctElectrodeValues, list: shortList(info.electrodeValues, (v) => `#${num(v)}`) })
          : t("props.pa.distinct", { count: info.distinctElectrodeValues });
  const vDigits = Math.min(4, Math.max(0, 2 - Math.floor(Math.log10(Math.max(info.potentialMax - info.potentialMin, 1e-9)))));
  const percent = (100 * info.electrodePoints) / info.points;

  return (
    <>
      <SimulationSections o={o} />
      <SliceSection o={o} />
      <Section title={t("props.section.array")} defaultOpen={o.formatId === "paHash" && !o.refine}>
        <Field label={t("props.format")}>
          <ReadOnlyValue>{t(format.labelKey)}</ReadOnlyValue>
        </Field>
        <Field label={t("props.pa.geometry")} hint={kind === "3d" ? undefined : t(`pa.kindHint.${kind}`)}>
          <ReadOnlyValue>{t(`pa.kind.${kind}`)}</ReadOnlyValue>
        </Field>
        <Field label={t("props.pa.grid")} hint={t("props.pa.points", { count: info.points })}>
          <ReadOnlyValue mono>{join([info.nx, info.ny, info.nz].slice(0, shown))}</ReadOnlyValue>
        </Field>
        <Field label={t("props.pa.cell")}>
          <ReadOnlyValue mono>{cell}</ReadOnlyValue>
        </Field>
        <Field label={t("props.pa.size")}>
          <ReadOnlyValue mono>{extent}</ReadOnlyValue>
        </Field>
        <Field label={t("props.pa.mirror")}>
          <ReadOnlyValue>{mirrorText}</ReadOnlyValue>
        </Field>
        <Field label={t("props.pa.electrodes")}>
          <span title={electrodes} className={styles.clip}>
            <ReadOnlyValue>{electrodes}</ReadOnlyValue>
          </span>
        </Field>
        <Field label={t("props.pa.electrodePoints")}>
          <ReadOnlyValue mono>
            {t("props.pa.electrodePointsValue", { count: info.electrodePoints, percent: Number(percent.toFixed(1)) })}
          </ReadOnlyValue>
        </Field>
        <Field label={t("props.pa.range")}>
          <ReadOnlyValue mono>
            {t("props.pa.rangeValue", { min: num(info.potentialMin, vDigits), max: num(info.potentialMax, vDigits) })}
          </ReadOnlyValue>
        </Field>
        <Field label={t("props.pa.field")}>
          <ReadOnlyValue>{t(`props.pa.field.${info.field}`)}</ReadOnlyValue>
        </Field>
        <Field label={t("props.pa.refinable")}>
          <ReadOnlyValue>{t(info.refinable ? "props.pa.yes" : "props.pa.no")}</ReadOnlyValue>
        </Field>
        <Field label={t("props.pa.header")}>
          <ReadOnlyValue>{t(info.mode === -1 ? "props.pa.header.standard" : "props.pa.header.extended")}</ReadOnlyValue>
        </Field>
        <Field label={t("props.fileSize")}>
          <ReadOnlyValue mono>{bytes(o.fileSize)}</ReadOnlyValue>
        </Field>
        {o.formatId === "paHash" && (
          <Notice title={t("props.pa.unrefined.title")} body={t("props.pa.unrefined.body")} />
        )}
        {info.pa0 && (
          <Notice
            title={t("props.pa.fastAdjust.title")}
            body={t("props.pa.fastAdjust.body", {
              list: shortList(info.pa0.electrodeNumbers, (v) => `#${v}`),
              ref: num(info.pa0.referenceVoltage),
            })}
          />
        )}
      </Section>
      <Section title={t("props.section.display")}>
        {Object.values(displayMirrors(info, true)).some(Boolean) && (
          <Field label={t("props.pa.showMirrors")} hint={t("props.pa.showMirrorsHint")}>
            <Switch
              checked={o.showMirrors}
              label={t("props.pa.showMirrors")}
              onChange={(v) => useApp.getState().setShowMirrors(o.id, v)}
            />
          </Field>
        )}
        <Field label={t("props.pa.surface")}>
          <ReadOnlyValue mono>{num(o.surface.count)}</ReadOnlyValue>
        </Field>
        {o.surface.step > 1 && <p className={styles.hint}>{t("props.pa.coarse", { step: o.surface.step })}</p>}
        <ObjectActions o={o} focus />
      </Section>
    </>
  );
}

function SliceSection({ o }: { o: PaObject }) {
  const { t, num } = useI18n();
  const { setSlice } = useApp.getState();
  const { info, slice } = o;
  const kind = paKind(info);
  const axes = sliceAxes(info);
  const d = spacing(info)[slice.axis];
  const [lo, hi] = coordRange(info, slice.axis, o.showMirrors);
  const digits = Math.min(4, Math.max(0, 1 - Math.floor(Math.log10(d))));
  const step = contourStep(info.potentialMin, info.potentialMax, slice.density);
  const pickAxis = (axis: Axis) => {
    const [a, b] = coordRange(info, axis, o.showMirrors);
    setSlice(o.id, { axis, coord: a < 0 ? 0 : Math.round((a + b) / 2) });
  };

  return (
    <Section title={t("props.section.slice")}>
      <Field label={t("props.slice.show")}>
        <Switch checked={slice.visible} label={t("props.slice.show")} onChange={(visible) => setSlice(o.id, { visible })} />
      </Field>
      {slice.visible && (
        <>
          {kind === "3d" ? (
            <>
              <Field label={t("props.slice.plane")}>
                <SegmentedControl<string>
                  ariaLabel={t("props.slice.plane")}
                  value={String(slice.axis)}
                  onChange={(v) => pickAxis(Number(v) as Axis)}
                  options={axes.map((a) => ({
                    value: String(a),
                    label: PLANE_NAMES[a],
                    title: t("props.slice.axisTitle", { axis: AXIS_NAMES[a], plane: PLANE_NAMES[a] }),
                  }))}
                />
              </Field>
              <NumberField
                label={`${t("props.slice.position")} ${AXIS_NAMES[slice.axis]}`}
                unit="mm"
                value={slice.coord * d}
                step={d}
                precision={digits}
                min={lo * d}
                max={hi * d}
                scrubTitle={t("props.scrubHint")}
                onChange={(v) => setSlice(o.id, { coord: Math.round(v / d) })}
              />
              <input
                type="range"
                className={`${inputs.slider} ${inputs.sliderPlain}`}
                min={lo}
                max={hi}
                step={1}
                value={slice.coord}
                aria-label={t("props.slice.position")}
                onChange={(e) => setSlice(o.id, { coord: Number(e.target.value) })}
              />
              <div className={styles.scale}>
                <span className="num">{num(lo * d, digits)}</span>
                <span className="num">{num(hi * d, digits)} mm</span>
              </div>
            </>
          ) : (
            <Field label={t("props.slice.plane")} hint={t("props.slice.fixedHint")}>
              <ReadOnlyValue>{t(kind === "cylindrical" ? "props.slice.fixed.cylindrical" : "props.slice.fixed.planar2d")}</ReadOnlyValue>
            </Field>
          )}
          <Field label={t("props.slice.contours")}>
            <Switch checked={slice.contours} label={t("props.slice.contours")} onChange={(contours) => setSlice(o.id, { contours })} />
          </Field>
          {slice.contours && (
            <Field label={t("props.slice.density")} hint={step > 0 ? t("props.slice.step", { step }) : undefined}>
              <SegmentedControl<ContourDensity>
                ariaLabel={t("props.slice.density")}
                value={slice.density}
                onChange={(density) => setSlice(o.id, { density })}
                options={(["few", "normal", "many"] as const).map((v) => ({ value: v, label: t(`props.slice.density.${v}`) }))}
              />
            </Field>
          )}
          <Field label={t("props.slice.cut")} hint={t("props.slice.cutHint")}>
            <Switch checked={slice.cut} label={t("props.slice.cut")} onChange={(cut) => setSlice(o.id, { cut })} />
          </Field>
          <p className={styles.hint}>{t("props.slice.hoverHint")}</p>
        </>
      )}
    </Section>
  );
}

function Notice({ title, body }: { title: string; body: string }) {
  return (
    <div className={styles.notice}>
      <Info className={styles.noticeIcon} aria-hidden />
      <div>
        <p className={styles.noticeTitle}>{title}</p>
        <p className={styles.noticeBody}>{body}</p>
      </div>
    </div>
  );
}

function ObjectActions({ o, focus }: { o: SceneObject; focus?: boolean }) {
  const { t } = useI18n();
  const { requestView, removeObject, log } = useApp.getState();
  return (
    <div className={styles.actions}>
      {focus && (
        <Button size="sm" icon={<Crosshair />} onClick={() => requestView({ type: "fit", ids: [o.id] })}>
          {t("props.focus")}
        </Button>
      )}
      <Button
        size="sm"
        variant="ghost"
        icon={<Trash2 />}
        onClick={() => {
          removeObject(o.id);
          log("info", { key: "log.removed", params: { name: objectName(o) } });
        }}
      >
        {t("props.remove")}
      </Button>
    </div>
  );
}

/** One electrode of a potential array (selected in the scene tree). */
function ElectrodeProperties({ o, e }: { o: PaObject; e: PaElectrode }) {
  const { t, num } = useI18n();
  const name = electrodeName(o, e, t, num);
  const [text, setText] = useState(name);
  useEffect(() => setText(name), [name]);
  const hidden = o.hiddenElectrodes.includes(e.label);
  const { renameElectrode, toggleElectrode, select } = useApp.getState();
  const kindKey: MessageKey =
    e.kind === "adjustable" ? "props.electrode.kind.adjustable" : e.kind === "numbered" ? "props.electrode.kind.numbered" : e.kind === "potential" ? "props.electrode.kind.potential" : "props.electrode.kind.fixed";
  return (
    <>
      <div className={styles.identity}>
        <span className={styles.identityIcon}>
          <Zap />
        </span>
        <div className={styles.identityText}>
          <p className={styles.identityName}>{name}</p>
          <p className={styles.identityType}>
            {e.number !== null ? t("props.electrode.typeNumbered", { n: e.number, file: objectName(o) }) : t("props.electrode.type", { file: objectName(o) })}
          </p>
        </div>
      </div>
      <Section title={t("props.section.general")}>
        <Field label={t("props.name")} htmlFor="prop-electrode-name" hint={t("props.electrode.nameHint")}>
          <input
            id="prop-electrode-name"
            className={inputs.input}
            value={text}
            onChange={(ev) => setText(ev.target.value)}
            onBlur={() => text !== name && renameElectrode(o.id, e.label, text)}
            onKeyDown={(ev) => {
              if (ev.key === "Enter") ev.currentTarget.blur();
              if (ev.key === "Escape") setText(name);
            }}
          />
        </Field>
        <Field label={t("props.visible")}>
          <Switch checked={!hidden} label={t("props.visible")} onChange={() => toggleElectrode(o.id, e.label)} />
        </Field>
        <Field label={t("props.type")}>
          <ReadOnlyValue>{t(kindKey)}</ReadOnlyValue>
        </Field>
        {e.points !== null && (
          <Field label={t("props.pa.electrodePoints")}>
            <ReadOnlyValue mono>{num(e.points)}</ReadOnlyValue>
          </Field>
        )}
      </Section>
      <Section title={t("props.section.potential")}>
        {e.kind === "adjustable" && simulationAvailable() ? (
          <VoltageControl o={o} electrode={e} label={t("props.voltage")} />
        ) : e.voltage !== null ? (
          <Field label={t("props.voltage")}>
            <ReadOnlyValue mono>{num(e.voltage)} V</ReadOnlyValue>
          </Field>
        ) : null}
        <p className={styles.hint}>{t(`props.electrode.hint.${e.kind}`)}</p>
        <div className={styles.actions}>
          <Button size="sm" icon={<ArrowUpLeft />} onClick={() => select(o.id)}>
            {t("props.electrode.array")}
          </Button>
        </div>
      </Section>
    </>
  );
}
