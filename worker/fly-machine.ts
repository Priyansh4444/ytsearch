export type FlyMachineState = string;

type FlyEnvironment = {
  FLY_API_BASE_URL: string;
  FLYIO_TOKEN: string;
};

export async function flyMachineState(
  env: FlyEnvironment,
  appName: string,
  machineId: string,
): Promise<FlyMachineState> {
  const response = await fetch(machineUrl(env, appName, machineId), {
    headers: flyHeaders(env),
  });
  if (!response.ok) throw new Error(`Fly machine lookup failed with HTTP ${response.status}`);
  const machine = await response.json<{ state?: string }>();
  return machine.state ?? "unknown";
}

export async function startFlyMachine(
  env: FlyEnvironment,
  appName: string,
  machineId: string,
): Promise<void> {
  const response = await fetch(`${machineUrl(env, appName, machineId)}/start`, {
    method: "POST",
    headers: flyHeaders(env),
  });
  if (!response.ok) throw new Error(`Fly machine start failed with HTTP ${response.status}`);
}

export async function stopFlyMachine(
  env: FlyEnvironment,
  appName: string,
  machineId: string,
): Promise<void> {
  const response = await fetch(`${machineUrl(env, appName, machineId)}/stop`, {
    method: "POST",
    headers: { ...flyHeaders(env), "Content-Type": "application/json" },
    body: JSON.stringify({ signal: "SIGINT", timeout: "120" }),
  });
  if (!response.ok) throw new Error(`Fly machine stop failed with HTTP ${response.status}`);
}

function machineUrl(env: FlyEnvironment, appName: string, machineId: string): string {
  return `${env.FLY_API_BASE_URL}/v1/apps/${encodeURIComponent(appName)}/machines/${encodeURIComponent(machineId)}`;
}

function flyHeaders(env: FlyEnvironment): { Authorization: string } {
  return { Authorization: `Bearer ${env.FLYIO_TOKEN}` };
}
