const DEFAULT_PUBLIC_APP_ORIGIN = "https://search.pronsh.dev";

export function requestOriginAllowed(request: URL, configuredOrigin?: string): boolean {
  if (isLocalHost(request.hostname)) return true;

  return request.origin === canonicalOrigin(configuredOrigin);
}

// The binding is fixed for an isolate's lifetime, so parse it once rather
// than on every request.
let canonicalOriginCache: { configured: string | undefined; origin: string } | undefined;

function canonicalOrigin(configuredOrigin?: string): string {
  if (!canonicalOriginCache || canonicalOriginCache.configured !== configuredOrigin) {
    canonicalOriginCache = {
      configured: configuredOrigin,
      origin: parseCanonicalOrigin(configuredOrigin),
    };
  }
  return canonicalOriginCache.origin;
}

function parseCanonicalOrigin(configuredOrigin?: string): string {
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
