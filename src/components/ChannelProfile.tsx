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
  size?: "compact" | "default";
}> = (props) => {
  const [imageFailed, setImageFailed] = createSignal(false);
  const sizeClass = () => (props.size === "compact" ? "size-9" : "size-11");
  const initials = () =>
    (props.channel?.name ?? props.channel?.handle ?? "?").slice(0, 2).toUpperCase();
  const avatarUrl = () => {
    if (imageFailed() || !props.channel?.channel_id) return undefined;
    return `${API_BASE_URL}/api/channel-images/${encodeURIComponent(props.channel.channel_id)}`;
  };

  return (
    <Show
      when={avatarUrl()}
      fallback={
        <div
          class={`${sizeClass()} grid shrink-0 place-items-center border border-vesper-border bg-vesper-panel2 text-xs font-semibold text-vesper-yellow`}
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
          class={`${sizeClass()} shrink-0 border border-vesper-border object-cover`}
          loading={props.size === "compact" ? "lazy" : "eager"}
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
  <div class="flex min-w-0 items-center gap-3">
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
