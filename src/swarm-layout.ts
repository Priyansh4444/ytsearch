type SwarmPosition = {
  x: number;
  y: number;
  color: string;
};

const VESPER_SWARM_COLORS = ["#ffc799", "#99ffe4", "#8db4ff", "#ff8080", "#ffb86c"] as const;

/** Position `index` of `total` alternately on two concentric oval rings. */
export function ovalSwarmPosition(index: number, total: number): SwarmPosition {
  const ring = index % 2;
  const ringCount = ring === 0 ? Math.ceil(total / 2) : Math.floor(total / 2);
  const angle = (Math.floor(index / 2) / ringCount) * Math.PI * 2 - Math.PI / 2 + ring * 0.09;
  const radiusX = ring === 0 ? 44 : 36;
  const radiusY = ring === 0 ? 43 : 35;
  return {
    x: round(50 + Math.cos(angle) * radiusX),
    y: round(50 + Math.sin(angle) * radiusY),
    color: VESPER_SWARM_COLORS[index % VESPER_SWARM_COLORS.length] ?? VESPER_SWARM_COLORS[0],
  };
}

export function magneticOffset(deltaX: number, deltaY: number) {
  // Squared-distance early-out first: identical outputs (d2 >= 220^2 iff
  // d >= 220; pull rounds to 0 at the boundary either way), no sqrt per
  // far item per active pointer frame.
  const d2 = deltaX * deltaX + deltaY * deltaY;
  if (d2 >= 220 * 220) return { x: 0, y: 0 };
  const distance = Math.sqrt(d2);
  const pull = (1 - distance / 220) ** 2 * 0.6;
  return { x: round(deltaX * pull), y: round(deltaY * pull) };
}

function round(value: number): number {
  return Math.round(value * 100) / 100;
}
