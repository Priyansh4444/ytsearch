import { shouldWakeIndexer } from "./wake-policy";
import { requestOriginAllowed } from "./origin-policy";

type WakeMessage = {
  reason: "index-request";
};

export default {
  async fetch(request, env): Promise<Response> {
    const publicAppOrigin = (env as Env & { PUBLIC_APP_ORIGIN?: string }).PUBLIC_APP_ORIGIN;
    if (!requestOriginAllowed(request.url, publicAppOrigin)) {
      return new Response("Forbidden", {
        status: 403,
        headers: { "Cache-Control": "no-store" },
      });
    }
    const url = new URL(request.url);
    const isApiRequest = url.pathname === "/api" || url.pathname.startsWith("/api/");

    if (!isApiRequest) {
      return env.ASSETS.fetch(request);
    }

    const upstreamUrl = new URL(url.pathname + url.search, env.API_BASE_URL);
    const upstreamResponse = await fetch(new Request(upstreamUrl, request));
    if (shouldWakeIndexer(request.method, url.pathname, upstreamResponse.ok)) {
      await env.INDEX_WAKE_QUEUE.send({ reason: "index-request" });
    }
    return new Response(upstreamResponse.body, upstreamResponse);
  },

  async queue(batch, env): Promise<void> {
    for (const message of batch.messages) {
      try {
        await startIndexer(env);
        message.ack();
      } catch (error) {
        // eslint-disable-next-line no-console -- Workers structured log event.
        console.error(
          JSON.stringify({
            event: "indexer_wake_failed",
            attempt: message.attempts,
            error: error instanceof Error ? error.message : String(error),
          }),
        );
        message.retry({ delaySeconds: retryDelaySeconds(message.attempts) });
      }
    }
  },

  async scheduled(_controller, env): Promise<void> {
    await startIndexer(env);
  },
} satisfies ExportedHandler<Env, WakeMessage>;

async function startIndexer(env: Env): Promise<void> {
  const machineUrl = `${env.FLY_API_BASE_URL}/v1/apps/${env.FLY_APP_NAME}/machines/${env.FLY_MACHINE_ID}`;
  const headers = { Authorization: `Bearer ${env.FLYIO_TOKEN}` };
  const machineResponse = await fetch(machineUrl, { headers });
  if (!machineResponse.ok) {
    throw new Error(`Fly machine lookup failed with HTTP ${machineResponse.status}`);
  }

  const machine = await machineResponse.json<{ state?: string }>();
  if (machine.state === "started" || machine.state === "starting") return;

  const startResponse = await fetch(`${machineUrl}/start`, { method: "POST", headers });
  if (!startResponse.ok) {
    throw new Error(`Fly machine start failed with HTTP ${startResponse.status}`);
  }

  // eslint-disable-next-line no-console -- Workers structured log event.
  console.log(JSON.stringify({ event: "indexer_woken", machine: env.FLY_MACHINE_ID }));
}

function retryDelaySeconds(attempts: number): number {
  return Math.min(30 * 2 ** Math.max(0, attempts - 1), 3_600);
}
