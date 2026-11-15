type DashboardSecrets = {
  DASHBOARD_APPROVAL_PASSKEY?: string;
  DASHBOARD_QUEUE_TOKEN?: string;
};

function failure(status: number, error: string, allow?: string): Response {
  return Response.json(
    { error },
    {
      status,
      headers: { "Cache-Control": "no-store", ...(allow ? { Allow: allow } : {}) },
    },
  );
}

export async function dashboardApprovalRequest(
  request: Request,
  env: DashboardSecrets,
): Promise<Request | Response> {
  const url = new URL(request.url);
  const list = url.pathname === "/api/dashboard/index-requests";
  const approval = /^\/api\/dashboard\/index-requests\/[^/]+\/approve$/.test(url.pathname);
  if (!list && !approval) return failure(404, "Unknown dashboard operation");
  const method = list ? "GET" : "POST";
  if (request.method !== method) return failure(405, "Method not allowed", method);
  if (!env.DASHBOARD_APPROVAL_PASSKEY || !env.DASHBOARD_QUEUE_TOKEN) {
    return failure(503, "Dashboard approvals are not configured");
  }
  const authorization = request.headers.get("Authorization");
  const supplied = authorization?.startsWith("Bearer ") ? authorization.slice(7) : "";
  // Hash first so the timing-safe comparison always receives equal-length buffers.
  const encoder = new TextEncoder();
  const [actual, expected] = await Promise.all([
    crypto.subtle.digest("SHA-256", encoder.encode(supplied)),
    crypto.subtle.digest("SHA-256", encoder.encode(env.DASHBOARD_APPROVAL_PASSKEY)),
  ]);
  if (!crypto.subtle.timingSafeEqual(actual, expected)) {
    return failure(401, "Invalid approval password");
  }
  url.pathname = url.pathname.replace("/api/dashboard/", "/api/");
  url.search = list ? "?limit=1000" : "";
  const headers = new Headers(request.headers);
  headers.set("Authorization", `Bearer ${env.DASHBOARD_QUEUE_TOKEN}`);
  headers.delete("x-index-queue-token");
  return new Request(url, { method, headers });
}
