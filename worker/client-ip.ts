// The API rate-limits each visitor by IP. Anyone can send `cf-connecting-ip`
// straight to the public Fly hostname, so the API trusts a visitor IP only
// alongside this shared key, which only the Worker holds (the `API_PROXY_KEY`
// secret here, `YTSEARCH_API_PROXY_KEY` on the API).
export const CLIENT_IP_HEADER = "x-ytsearch-client-ip";
export const PROXY_KEY_HEADER = "x-ytsearch-proxy-key";

export function stampClientIp(upstream: Request, visitor: Request, proxyKey?: string): void {
  // Never forward values a visitor supplied for these headers.
  upstream.headers.delete(CLIENT_IP_HEADER);
  upstream.headers.delete(PROXY_KEY_HEADER);
  const ip = visitor.headers.get("cf-connecting-ip");
  if (!proxyKey || !ip) return;
  upstream.headers.set(CLIENT_IP_HEADER, ip);
  upstream.headers.set(PROXY_KEY_HEADER, proxyKey);
}
