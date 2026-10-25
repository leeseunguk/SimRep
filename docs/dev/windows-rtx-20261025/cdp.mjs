// Windows Node 24, WebView2 CDP. This records verification evidence, not app code.
import { readFileSync, writeFileSync } from "node:fs";

const [mode, argument, output] = process.argv.slice(2);
const pages = await (await fetch(`http://127.0.0.1:${process.env.CDP_PORT || 9333}/json/list`)).json();
const page = pages.find((p) => p.type === "page");
if (!page) throw new Error("No WebView2 page");
const ws = new WebSocket(page.webSocketDebuggerUrl);
const pending = new Map();
let id = 0;
ws.onmessage = (event) => {
  const message = JSON.parse(event.data);
  const resolve = pending.get(message.id);
  if (resolve) {
    pending.delete(message.id);
    resolve(message);
  }
};
await new Promise((resolve, reject) => {
  ws.onopen = resolve;
  ws.onerror = reject;
});
const send = (method, params = {}) => new Promise((resolve) => {
  const next = ++id;
  pending.set(next, resolve);
  ws.send(JSON.stringify({ id: next, method, params }));
});
const evaluate = async (body) => {
  const message = await send("Runtime.evaluate", {
    expression: `(async () => { ${body} })()`, awaitPromise: true, returnByValue: true,
  });
  if (message.error || message.result?.exceptionDetails) {
    throw new Error(JSON.stringify(message.error || message.result.exceptionDetails));
  }
  return message.result.result.value;
};
try {
  await send("Emulation.setDeviceMetricsOverride", { width: 1440, height: 900, deviceScaleFactor: 1, mobile: false });
  if (mode === "shot") {
    const shot = await send("Page.captureScreenshot", { format: "png" });
    writeFileSync(argument, Buffer.from(shot.result.data, "base64"));
    console.log("SCREENSHOT", argument);
  } else {
    const body = mode === "file" ? readFileSync(argument, "utf8") : argument;
    const value = await evaluate(body);
    const text = JSON.stringify(value, null, 2);
    if (output) writeFileSync(output, `${text}\n`);
    console.log(typeof value === "string" ? value : text);
  }
} finally {
  ws.close();
}
