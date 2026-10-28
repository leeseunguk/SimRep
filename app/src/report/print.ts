import { isTauri } from "@tauri-apps/api/core";

/**
 * PDF through the desktop WebView's print-to-PDF: the report is loaded into an off-screen iframe
 * of this webview and printed from there (WebView2 / WebKitGTK open the system print dialog, whose
 * "Save as PDF" / "Microsoft Print to PDF" printer writes the A4 PDF; the report's @page CSS sets
 * the paper). Only the iframe is printed, not the app around it.
 */

/** Whether this runtime can print the report (desktop app with a print-capable webview). */
export function canPrintPdf(): boolean {
  return isTauri() && typeof window !== "undefined" && typeof window.print === "function";
}

let frame: HTMLIFrameElement | null = null;

/** Opens the print dialog for `html`; rejects when the webview cannot print. */
export function printReport(html: string): Promise<void> {
  if (!canPrintPdf()) return Promise.reject(new Error("print-to-PDF is not available in this runtime"));
  frame?.remove();
  const f = document.createElement("iframe");
  frame = f;
  f.setAttribute("aria-hidden", "true");
  f.tabIndex = -1;
  // Off-screen but laid out (a display:none frame does not print in every engine).
  Object.assign(f.style, { position: "fixed", left: "-10000px", top: "0", width: "210mm", height: "297mm", border: "0" });
  return new Promise((resolve, reject) => {
    f.onload = () => {
      const w = f.contentWindow;
      if (!w || typeof w.print !== "function") {
        f.remove();
        reject(new Error("the webview has no print support"));
        return;
      }
      w.addEventListener("afterprint", () => {
        if (frame === f) frame = null;
        f.remove();
      });
      try {
        w.focus();
        w.print();
        resolve();
      } catch (err) {
        f.remove();
        reject(err instanceof Error ? err : new Error(String(err)));
      }
    };
    f.srcdoc = html;
    document.body.appendChild(f);
  });
}
