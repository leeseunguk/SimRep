// Scroll the properties panel so the element whose own text starts with __args.text sits __args.pad px
// below the scroller top (like scrollto.js), then return CSS-px boxes [left, top, right, bottom] of the
// right panel and of each __args.probe text (plus its closest button/card ancestor) for cropping.
const a = __args;
const own = (e) => [...e.childNodes].filter((n) => n.nodeType === 3).map((n) => n.textContent).join('').trim();
const find = (t) => [...document.querySelectorAll('aside *')].find((e) => own(e).startsWith(t));
const box = (e) => { const b = e.getBoundingClientRect(); return [b.left, b.top, b.right, b.bottom].map((v) => Math.round(v * 100) / 100); };
const el = find(a.text);
let p = el.parentElement;
while (p && !(p.scrollHeight > p.clientHeight && /auto|scroll/.test(getComputedStyle(p).overflowY))) p = p.parentElement;
p.scrollTop += el.getBoundingClientRect().top - p.getBoundingClientRect().top - (a.pad ?? 4);
await new Promise((r) => setTimeout(r, 400));
const out = { aside: box([...document.querySelectorAll('aside')].at(-1)), scroller: box(p) };
for (const t of a.probe ?? []) {
  const e = find(t);
  if (!e) { out[t] = null; continue; }
  const up = [];
  for (let q = e, i = 0; q && i < (a.depth ?? 4); q = q.parentElement, i++) up.push([q.tagName, q.className?.toString().slice(0, 40), box(q)]);
  out[t] = up;
}
return out;
