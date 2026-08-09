import { Show, createSignal } from "solid-js";
import type { Component } from "solid-js";
import { API_BASE_URL } from "../api";
import type { IndexedChannel } from "../types";

export const indexedCountLabel = (channel: IndexedChannel) => {
  const total = Math.max(channel.public_video_count, channel.indexed_video_count);
  return `${channel.indexed_video_count} / ${total} videos indexed`;
};

export const ChannelAvatar: Component<{
  channel: IndexedChannel | undefined;
  size?: "swarm" | "compact" | "default";
  transitionName?: string;
}> = (props) => {
  const [imageFailed, setImageFailed] = createSignal(false);
  const sizeClass = () => {
    if (props.size === "swarm") return "size-10 sm:size-12";
    return props.size === "compact" ? "size-9" : "size-11";
  };
  const outlineClass = () => (props.size === "swarm" ? "" : "ring-1 ring-white/10");
  const transitionStyle = () =>
    props.transitionName ? `view-transition-name: ${props.transitionName}` : undefined;
  const initials = () =>
    (props.channel?.name ?? props.channel?.handle ?? "?").slice(0, 2).toUpperCase();
  const avatarUrl = () => {
    const channel = props.channel;
    if (imageFailed() || !channel?.channel_id || !channel.avatar_url) return undefined;
    return `${API_BASE_URL}/api/channel-images/${encodeURIComponent(channel.channel_id)}?v=${imageVersion(channel.avatar_url)}&s=96`;
  };

  return (
    <Show
      when={avatarUrl()}
      fallback={
        <div
          class={`${sizeClass()} ${outlineClass()} grid aspect-square shrink-0 place-items-center rounded-full bg-vesper-panel2 text-xs font-semibold text-vesper-yellow`}
          style={transitionStyle()}
          aria-label={`${props.channel?.name ?? "Channel"} avatar unavailable`}
        >
          {initials()}
        </div>
      }
    >
      {(avatarUrl) => (
        <img
          src={avatarUrl()}
          alt={`${props.channel?.name ?? "Channel"} profile picture`}
          width="96"
          height="96"
          class={`${sizeClass()} ${outlineClass()} aspect-square max-w-none shrink-0 rounded-full object-cover`}
          style={transitionStyle()}
          loading={props.size === "compact" ? "lazy" : "eager"}
          decoding="async"
          onError={() => setImageFailed(true)}
        />
      )}
    </Show>
  );
};

export const ChannelIdentity: Component<{
  channel: IndexedChannel | undefined;
  handle: string;
}> = (props) => (
  <div
    class="flex min-w-0 items-center gap-3"
    style="view-transition-name: selected-channel-identity"
  >
    <ChannelAvatar channel={props.channel} />
    <div class="min-w-0">
      <p class="truncate text-lg font-semibold text-white">{props.channel?.name ?? props.handle}</p>
      <p class="truncate text-sm text-vesper-muted">
        {props.channel?.handle ?? props.handle}
        <Show when={props.channel}> · {props.channel ? indexedCountLabel(props.channel) : ""}</Show>
      </p>
    </div>
  </div>
);

function imageVersion(url: string): string {
  let hash = 2_166_136_261;
  for (const character of url) {
    hash ^= character.charCodeAt(0);
    hash = Math.imul(hash, 16_777_619);
  }
  return (hash >>> 0).toString(36);
}
