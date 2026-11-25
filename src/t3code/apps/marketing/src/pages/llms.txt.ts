import type { APIRoute } from "astro";

import { absoluteUrl } from "../lib/seo";
import { UPSTREAM_REPOSITORY_URL } from "../lib/site";

export const GET: APIRoute = () =>
  new Response(
    `# VikEA

Desktop workspace for coding agents with CC Switch. VikEA is an unofficial fork based on the T3 Code source code.

- [VikEA downloads](${absoluteUrl("/download")})
- [Licensing and attribution](${absoluteUrl("/legal")})
- [Upstream T3 Code source](${UPSTREAM_REPOSITORY_URL})
`,
    { headers: { "Content-Type": "text/plain; charset=utf-8" } },
  );
