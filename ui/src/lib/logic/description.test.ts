import { describe, expect, it } from "vitest";
import { buildDescriptionDoc, externalHref } from "./description";

describe("buildDescriptionDoc", () => {
  const doc = buildDescriptionDoc("<p>Hola</p><script>alert(1)</script>", {
    base: "http://127.0.0.1:9/m/tok/Skool/G/L/",
    mediaOrigin: "http://127.0.0.1:9",
    css: "body{color:red}",
  });

  it("starts with a restrictive CSP before any lesson markup", () => {
    const csp = doc.indexOf('http-equiv="Content-Security-Policy"');
    expect(csp).toBeGreaterThan(-1);
    expect(csp).toBeLessThan(doc.indexOf("<p>Hola</p>"));
    expect(doc).toContain("default-src 'none'");
    expect(doc).toContain("img-src http://127.0.0.1:9 data:");
    expect(doc).not.toMatch(/script-src[^;]*'unsafe/);
  });

  it("resolves relative assets against the lesson folder on the media server", () => {
    expect(doc).toContain('<base href="http://127.0.0.1:9/m/tok/Skool/G/L/" target="_blank">');
  });

  it("injects the theme stylesheet and keeps the content", () => {
    expect(doc).toContain("<style>body{color:red}</style>");
    expect(doc).toContain("<p>Hola</p>");
  });

  it("escapes attribute values", () => {
    const d = buildDescriptionDoc("", { base: 'x" onload="y', mediaOrigin: "o", css: "" });
    expect(d).toContain('href="x&quot; onload=&quot;y"');
  });

  it("omits the media origin when there is none (preview)", () => {
    const d = buildDescriptionDoc("", { base: "", mediaOrigin: "", css: "" });
    expect(d).toContain("img-src data:");
    expect(d).not.toContain("<base");
  });
});

describe("externalHref", () => {
  it("accepts http, https and mailto links only", () => {
    expect(externalHref("https://a.b/c")).toBe("https://a.b/c");
    expect(externalHref("HTTP://a.b")).toBe("HTTP://a.b");
    expect(externalHref("mailto:x@y.z")).toBe("mailto:x@y.z");
    expect(externalHref("javascript:alert(1)")).toBeNull();
    expect(externalHref("file:///etc/passwd")).toBeNull();
    expect(externalHref("")).toBeNull();
    expect(externalHref(null)).toBeNull();
  });
});
