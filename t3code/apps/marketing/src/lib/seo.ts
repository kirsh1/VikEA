import { DESKTOP_DOWNLOAD_URL, SITE_ORIGIN } from "./site";

export type JsonLd = Record<string, unknown>;

export const absoluteUrl = (pathname: string): string => new URL(pathname, SITE_ORIGIN).href;

export const softwareJsonLd = (description: string): JsonLd => ({
  "@type": "SoftwareApplication",
  name: "VikEA",
  description,
  url: SITE_ORIGIN,
  applicationCategory: "DeveloperApplication",
  license: "https://opensource.org/license/mit",
  ...(DESKTOP_DOWNLOAD_URL ? { downloadUrl: DESKTOP_DOWNLOAD_URL } : {}),
});

export const jsonLdGraph = (nodes: ReadonlyArray<JsonLd>): string =>
  JSON.stringify({ "@context": "https://schema.org", "@graph": nodes }).replace(/</g, "\\u003c");

export const STATIC_PAGES = [
  { path: "/", priority: "1.0" },
  { path: "/download", priority: "0.9" },
  { path: "/legal", priority: "0.3" },
] as const;
