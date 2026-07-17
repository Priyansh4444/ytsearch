const DEFAULT_PUBLIC_APP_ORIGIN = "https://search.pronsh.dev";

export function requestOriginAllowed(requestUrl: string, configuredOrigin?: string): boolean {
  const request = new URL(requestUrl);
  if (isLocalHost(request.hostname)) return true;

  const allowed = canonicalOrigin(configuredOrigin);
  return request.origin === allowed;
}

function canonicalOrigin(configuredOrigin?: string): string {
  const value = configuredOrigin?.trim() || DEFAULT_PUBLIC_APP_ORIGIN;
  try {
    return new URL(value).origin;
  } catch {
    // A broken production binding must fail closed, not silently expose a
    // Pages preview or workers.dev hostname.
    return "invalid://configuration";
  }
}

function isLocalHost(hostname: string): boolean {
  return hostname === "localhost" || hostname === "127.0.0.1" || hostname === "[::1]";
}
