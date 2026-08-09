import { For, Show, createEffect, createMemo, onCleanup, onMount } from "solid-js";
import type { Component } from "solid-js";
import type { IndexedChannel } from "../types";
import { routeForHandle } from "../utils";
import { magneticOffset, ovalSwarmPositions } from "../swarm-layout";
import { ChannelAvatar } from "./channel-profile";

type SwarmItem = {
  channel: IndexedChannel;
  x: number;
  y: number;
  mobileX: number;
  mobileY: number;
  color: string;
};

export const ChannelPicker: Component<{
  channels: IndexedChannel[] | undefined;
  visible: boolean;
  onSelect: (handle: string) => void;
}> = (props) => {
  const swarm = createMemo(() => positionChannels(props.channels ?? []));
  const itemElements = new Map<string, HTMLAnchorElement>();
  let stage: HTMLDivElement | undefined;
  let animationFrame: number | undefined;
  let activeElements = new Set<HTMLAnchorElement>();

  const moveSwarm = (event: PointerEvent) => {
    if (event.pointerType === "touch" || !props.visible) return;
    if (animationFrame) window.cancelAnimationFrame(animationFrame);
    animationFrame = window.requestAnimationFrame(() => {
      if (!stage) return;
      const bounds = stage.getBoundingClientRect();
      const useMobileLayout = bounds.width < 640;
      stage.dataset.cursorActive = "true";
      stage.style.setProperty("--cursor-x", `${event.clientX - bounds.left}px`);
      stage.style.setProperty("--cursor-y", `${event.clientY - bounds.top}px`);
      const nextActiveElements = new Set<HTMLAnchorElement>();
      for (const item of swarm()) {
        const element = itemElements.get(item.channel.id);
        if (!element) continue;
        const xPercent = useMobileLayout ? item.mobileX : item.x;
        const yPercent = useMobileLayout ? item.mobileY : item.y;
        const centerX = bounds.left + (xPercent / 100) * bounds.width;
        const centerY = bounds.top + (yPercent / 100) * bounds.height;
        const offset = magneticOffset(event.clientX - centerX, event.clientY - centerY);
        if (offset.x === 0 && offset.y === 0) continue;
        nextActiveElements.add(element);
        element.style.willChange = "transform";
        element.style.setProperty("--magnet-x", `${offset.x}px`);
        element.style.setProperty("--magnet-y", `${offset.y}px`);
      }
      for (const element of activeElements) {
        if (nextActiveElements.has(element)) continue;
        resetMagnet(element);
      }
      activeElements = nextActiveElements;
    });
  };

  const releaseSwarm = () => {
    for (const element of activeElements) resetMagnet(element);
    activeElements.clear();
    if (stage) stage.dataset.cursorActive = "false";
  };

  onMount(() => {
    if (
      !window.matchMedia("(pointer: fine)").matches ||
      window.matchMedia("(prefers-reduced-motion: reduce)").matches
    ) {
      return;
    }
    // Track over the whole viewport so the magnetic field remains continuous
    // while the swarm canvas itself stays pointer-transparent.
    window.addEventListener("pointermove", moveSwarm, { passive: true });
    window.addEventListener("blur", releaseSwarm);
  });

  createEffect(() => {
    if (!props.visible) releaseSwarm();
  });

  onCleanup(() => {
    if (animationFrame) window.cancelAnimationFrame(animationFrame);
    window.removeEventListener("pointermove", moveSwarm);
    window.removeEventListener("blur", releaseSwarm);
  });

  return (
    <Show when={swarm().length > 0}>
      <div
        ref={(element) => (stage = element)}
        class={`channel-swarm ${props.visible ? "channel-swarm-visible" : "channel-swarm-hidden"}`}
        aria-label="Choose a YouTube channel"
        aria-hidden={!props.visible}
      >
        <For each={swarm()}>
          {(item) => (
            <a
              ref={(element) => itemElements.set(item.channel.id, element)}
              href={routeForHandle(item.channel.handle)}
              aria-label={`Search ${item.channel.name}`}
              title={`Search ${item.channel.name}`}
              class="channel-swarm-item group"
              style={`--swarm-x:${item.x}%;--swarm-y:${item.y}%;--swarm-mobile-x:${item.mobileX}%;--swarm-mobile-y:${item.mobileY}%;--swarm-color:${item.color}`}
              onClick={(event) => {
                event.preventDefault();
                event.currentTarget.style.viewTransitionName = "selected-channel-identity";
                props.onSelect(item.channel.handle);
              }}
            >
              <ChannelAvatar channel={item.channel} size="swarm" />
              <span class="channel-swarm-label">{item.channel.name}</span>
            </a>
          )}
        </For>
      </div>
    </Show>
  );
};

function positionChannels(channels: IndexedChannel[]): SwarmItem[] {
  const positions = ovalSwarmPositions(channels.length);
  return channels.map((channel, index) => {
    const position = positions[index];
    return {
      channel,
      ...position,
    };
  });
}

function resetMagnet(element: HTMLAnchorElement) {
  element.style.removeProperty("will-change");
  element.style.setProperty("--magnet-x", "0px");
  element.style.setProperty("--magnet-y", "0px");
}
