const STATUS_ERRORS: Readonly<Record<number, { code: string; error: string; hint: string }>> = {
  400: {
    code: "bad_request",
    error: "The request is invalid",
    hint: "Check the parameters against /openapi.json.",
  },
  404: {
    code: "not_found",
    error: "No such API route",
    hint: "See /docs or /openapi.json for the available routes.",
  },
  405: {
    code: "method_not_allowed",
    error: "This route does not accept that method",
    hint: "The public API is read-only; use GET.",
  },
  429: {
    code: "rate_limited",
    error: "Too many requests",
    hint: "Wait before retrying; search allows about 120 requests a minute per IP.",
  },
};

const FALLBACK_ERROR = {
  code: "unavailable",
  error: "The service could not answer",
  hint: "Retry shortly; /api/availability says whether search is serving.",
};

/**
 * Agents cannot act on an empty body or framework text, so every API failure leaves
 * as JSON. A failure that is already JSON passes through untouched.
 */
export async function errorForAgents(upstream: Response): Promise<Response> {
  if (upstream.ok || upstream.status < 400) return new Response(upstream.body, upstream);
  if ((upstream.headers.get("Content-Type") ?? "").includes("json")) {
    return new Response(upstream.body, upstream);
  }
  const detail = (await upstream.text()).trim();
  const known = STATUS_ERRORS[upstream.status] ?? FALLBACK_ERROR;
  const response = Response.json(
    { ...known, ...(detail && detail.length <= 500 ? { error: detail } : {}) },
    { status: upstream.status },
  );
  for (const name of ["Allow", "Retry-After"]) {
    const value = upstream.headers.get(name);
    if (value) response.headers.set(name, value);
  }
  return response;
}
