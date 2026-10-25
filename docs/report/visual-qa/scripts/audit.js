// DOM audit run in the app page after a capture (objective evidence that aims the review, not the verdict).
// Reports visible text under 12 CSS px, Hangul set in a monospace face, text below WCAG AA contrast
// (approximate: solid/alpha background layers composited up the tree, gradients and the 3D canvas ignored),
// interactive elements without an accessible name, pointer targets under 24 CSS px, and the scroll owners.
const HANGUL = /[\u3131-\u318e\uac00-\ud7a3]/;
const vis = (el) => {
  const r = el.getBoundingClientRect();
  if (r.width < 1 || r.height < 1 || r.bottom <= 0 || r.right <= 0 || r.top >= innerHeight || r.left >= innerWidth) return false;
  // Content of a closed <details> is not rendered even if it reports a box.
  const d = el.closest("details:not([open])");
  if (d && !el.closest("summary")) return false;
  for (let e = el; e; e = e.parentElement) {
    const cs = getComputedStyle(e);
    if (cs.visibility === "hidden" || cs.display === "none" || Number(cs.opacity) === 0) return false;
    if (e.classList?.contains("visually-hidden")) return false;
  }
  return true;
};
const parse = (c) => {
  let m = c.match(/rgba?\(([\d.]+)[ ,]+([\d.]+)[ ,]+([\d.]+)(?:[ ,/]+([\d.]+%?))?\)/);
  if (m) return [+m[1], +m[2], +m[3], m[4] === undefined ? 1 : m[4].endsWith("%") ? parseFloat(m[4]) / 100 : +m[4]];
  m = c.match(/color\(srgb ([\d.e-]+) ([\d.e-]+) ([\d.e-]+)(?: \/ ([\d.]+))?\)/);
  if (m) return [+m[1] * 255, +m[2] * 255, +m[3] * 255, m[4] === undefined ? 1 : +m[4]];
  return null;
};
const over = (top, bot) => {
  const a = top[3];
  return [top[0] * a + bot[0] * (1 - a), top[1] * a + bot[1] * (1 - a), top[2] * a + bot[2] * (1 - a), 1];
};
const root = parse(getComputedStyle(document.body).backgroundColor) ?? [255, 255, 255, 1];
const bgOf = (el) => {
  const layers = [];
  let opacity = 1;
  for (let e = el; e; e = e.parentElement) {
    const cs = getComputedStyle(e);
    opacity *= Number(cs.opacity);
    const c = parse(cs.backgroundColor);
    if (c && c[3] > 0) {
      layers.push(c);
      if (c[3] >= 1) break;
    }
  }
  let bg = root;
  for (let i = layers.length - 1; i >= 0; i--) bg = over(layers[i], bg);
  return { bg, opacity };
};
const lum = ([r, g, b]) => {
  const f = (x) => {
    x /= 255;
    return x <= 0.03928 ? x / 12.92 : ((x + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b);
};
const ratio = (a, b) => {
  const [x, y] = [lum(a), lum(b)];
  return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05);
};
const path = (el) => {
  const parts = [];
  for (let e = el; e && parts.length < 3 && e !== document.body; e = e.parentElement) {
    const cls = typeof e.className === "string" ? e.className.split(/\s+/)[0]?.replace(/_[A-Za-z0-9-]{5,}$/, "") : "";
    parts.unshift(e.tagName.toLowerCase() + (cls ? "." + cls : ""));
  }
  return parts.join(">");
};
const disabled = (el) => !!el.closest("[disabled], [aria-disabled=true], [inert]");

const small = [], mono = [], contrast = [], seen = new Set();
const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
for (let n = walker.nextNode(); n; n = walker.nextNode()) {
  const text = n.textContent.trim();
  if (!text) continue;
  const el = n.parentElement;
  if (!el || seen.has(el) || !vis(el) || el.closest("canvas, svg")) continue;
  seen.add(el);
  const cs = getComputedStyle(el);
  const fs = parseFloat(cs.fontSize);
  const where = path(el);
  if (fs < 12) small.push([text.slice(0, 40), fs, where]);
  // Hangul in a mono stack is only a defect when no proportional Korean face precedes the generic fallbacks.
  if (HANGUL.test(text) && /mono/i.test(cs.fontFamily.split(",")[0]) && !/Sans KR/.test(cs.fontFamily)) mono.push([text.slice(0, 40), where]);
  if (disabled(el) || el.closest("[class*=viewport], [class*=Viewport], [class*=legend], [class*=gizmo]")) continue;
  const fg = parse(cs.color);
  if (!fg) continue;
  const { bg, opacity } = bgOf(el);
  const eff = over([fg[0], fg[1], fg[2], fg[3] * opacity], bg);
  const large = fs >= 24 || (fs >= 18.66 && Number(cs.fontWeight) >= 700);
  const r = ratio(eff, bg);
  if (r < (large ? 3 : 4.5)) contrast.push([text.slice(0, 40), Math.round(r * 100) / 100, fs, where]);
}

const unnamed = [], targets = [];
for (const el of document.querySelectorAll("button, [role=button], a[href], input, select, textarea, [role=tab], [role=switch], [role=menuitem], [role=radio], [role=option]")) {
  if (!vis(el) || el.closest("[inert]")) continue;
  if (el.type === "hidden" || el.hidden) continue;
  const name = (el.getAttribute("aria-label") || el.getAttribute("aria-labelledby") && document.getElementById(el.getAttribute("aria-labelledby"))?.textContent || (el.labels && el.labels[0]?.textContent) || el.textContent || el.getAttribute("title") || el.getAttribute("placeholder") || "").trim();
  const visibleText = (el.textContent || "").trim();
  if (!name || /^[A-Z0-9]$/.test(name)) unnamed.push([el.tagName.toLowerCase(), name, path(el)]);
  const r = el.getBoundingClientRect();
  if (Math.min(r.width, r.height) < 24 && el.type !== "range") targets.push([visibleText.slice(0, 24) || el.getAttribute("aria-label") || el.title || el.type || "", Math.round(r.width), Math.round(r.height), path(el)]);
}

const scrollers = [...document.querySelectorAll("*")]
  .filter((el) => /auto|scroll/.test(getComputedStyle(el).overflowY) && el.scrollHeight > el.clientHeight && vis(el))
  .map((el) => path(el))
  .concat(document.scrollingElement.scrollHeight > innerHeight ? ["document"] : []);

const cap = (a) => ({ n: a.length, items: a.slice(0, 25) });
return { small: cap(small), monoHangul: cap(mono), lowContrast: cap(contrast), unnamed: cap(unnamed), smallTargets: cap(targets), scrollers };
