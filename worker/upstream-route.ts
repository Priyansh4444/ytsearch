const INDEX_QUEUE_ROUTES = ["/api/index-requests", "/api/index-status", "/api/index-activity"];

export function upstreamApiBase(
  pathname: string,
  searchApiBaseUrl: string,
  indexQueueApiBaseUrl?: string,
): string {
  return isIndexQueueRoute(pathname) && indexQueueApiBaseUrl
    ? indexQueueApiBaseUrl
    : searchApiBaseUrl;
}

export function isIndexQueueRoute(pathname: string): boolean {
  return INDEX_QUEUE_ROUTES.some((route) => pathname === route || pathname.startsWith(`${route}/`));
}
