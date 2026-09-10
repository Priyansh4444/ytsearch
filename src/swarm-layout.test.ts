import { describe, expect, it } from "vitest";
import { magneticOffset, ovalSwarmPositions } from "./swarm-layout";

describe("oval swarm layout", () => {
  it("places every creator on one of two safe oval rings", () => {
    const positions = ovalSwarmPositions(43);

    expect(positions).toHaveLength(43);
    for (const [index, position] of positions.entries()) {
      const ring = index % 2;
      const radiusX = ring === 0 ? 44 : 36;
      const radiusY = ring === 0 ? 43 : 35;
      const ovalDistance = ((position.x - 50) / radiusX) ** 2 + ((position.y - 50) / radiusY) ** 2;
      expect(Math.abs(ovalDistance - 1)).toBeLessThan(0.02);
      expect(position.x).toBeGreaterThanOrEqual(6);
      expect(position.x).toBeLessThanOrEqual(94);
      expect(position.y).toBeGreaterThanOrEqual(7);
      expect(position.y).toBeLessThanOrEqual(93);
    }
  });

  it("cycles through the complete Vesper accent palette", () => {
    expect(new Set(ovalSwarmPositions(10).map(({ color }) => color))).toEqual(
      new Set(["#ffc799", "#99ffe4", "#8db4ff", "#ff8080", "#ffb86c"]),
    );
  });
});

describe("swarm magnet", () => {
  it("pulls nearby avatars and leaves distant avatars still", () => {
    expect(magneticOffset(50, 0).x).toBeGreaterThan(0);
    expect(magneticOffset(220, 0)).toEqual({ x: 0, y: 0 });
  });

  it("keeps the pull symmetric and zero at and beyond the radius", () => {
    // Pins the squared-distance early-out: the boundary, the sign symmetry, and
    // the far field must match the previous Math.hypot version.
    expect(magneticOffset(0, 220)).toEqual({ x: 0, y: 0 });
    expect(magneticOffset(-50, 0)).toEqual({ x: -magneticOffset(50, 0).x, y: 0 });
    // At 219px the pull already rounds to zero, which is why the squared
    // early-out cannot change any output — it only skips the sqrt.
    expect(magneticOffset(219, 0)).toEqual({ x: 0, y: 0 });
    expect(magneticOffset(300, 0)).toEqual({ x: 0, y: 0 });
  });
});
