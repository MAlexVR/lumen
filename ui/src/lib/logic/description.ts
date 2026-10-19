// Wrapper document for lesson descriptions. The HTML is rendered through
// `srcdoc` in an iframe sandboxed without scripts; this wrapper adds a CSP
// that blocks scripts and network access except images from the local
// media server, a <base> so relative images resolve, and the dark theme.

export interface DescriptionDocOptions {
  /** Folder of the description on the media server, with trailing slash. */
  base: string;
  /** Origin of the media server (images may load from it). */
  mediaOrigin: string;
  css: string;
}

const attr = (s: string): string =>
  s.replace(/&/g, "&amp;").replace(/"/g, "&quot;").replace(/</g, "&lt;").replace(/>/g, "&gt;");

export function buildDescriptionDoc(html: string, opts: DescriptionDocOptions): string {
  const src = opts.mediaOrigin ? `${opts.mediaOrigin} data:` : "data:";
  const csp = `default-src 'none'; img-src ${src}; media-src ${src}; style-src 'unsafe-inline'; font-src 'none'; form-action 'none'`;
  return [
    "<!doctype html><html><head><meta charset=\"utf-8\">",
    `<meta http-equiv="Content-Security-Policy" content="${attr(csp)}">`,
    opts.base ? `<base href="${attr(opts.base)}" target="_blank">` : "",
    `<style>${opts.css}</style>`,
    "</head><body>",
    html,
    "</body></html>",
  ].join("");
}

/** Link targets the app may hand to the system browser. */
export function externalHref(href: string | null): string | null {
  return href && /^(https?:|mailto:)/i.test(href.trim()) ? href.trim() : null;
}
