// search.experimental.pronsh.dev previewed hybrid search and moment sharing; both
// are on search.pronsh.dev now. This Worker keeps its links (shared moments, embed
// cards) working by sending every request to the same path on production.
// Deployed by scripts/deploy-experimental.sh with wrangler.experimental.jsonc.
export const PRODUCTION_ORIGIN = "https://search.pronsh.dev";

export function redirectToProduction(request: Request): Response {
  const url = new URL(request.url);
  return Response.redirect(PRODUCTION_ORIGIN + url.pathname + url.search, 301);
}

export default { fetch: redirectToProduction };
