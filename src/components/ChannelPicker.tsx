import { For, Show } from "solid-js";
import type { Component } from "solid-js";
import type { IndexedChannel } from "../types";
import { normalizeHandle, routeForHandle } from "../utils";

export const ChannelPicker: Component<{
  channels: IndexedChannel[] | undefined;
  selectedHandle: string;
  onSelect: (handle: string) => void;
}> = (props) => (
  <Show when={(props.channels?.length ?? 0) > 0}>
    <div class="flex flex-wrap gap-2">
      <For each={props.channels}>
        {(channel) => {
          const selected = () =>
            normalizeHandle(channel.handle) === normalizeHandle(props.selectedHandle);
          return (
            <a
              href={routeForHandle(channel.handle)}
              onClick={(event) => {
                event.preventDefault();
                window.history.pushState({}, "", routeForHandle(channel.handle));
                props.onSelect(channel.handle);
              }}
              class={`group flex items-center gap-3 border px-3 py-2 transition duration-300 ${
                selected()
                  ? "border-vesper-yellow bg-vesper-panel2 text-white"
                  : "border-vesper-border text-vesper-muted hover:border-vesper-yellow hover:text-white"
              }`}
            >
              <ChannelAvatar channel={channel} />
              <span class="leading-tight">
                <span class="block text-sm font-medium">{channel.name}</span>
                <span class="block text-xs text-vesper-dim">
                  {channel.handle} · {channel.indexed_video_count} indexed
                </span>
              </span>
            </a>
          );
        }}
      </For>
    </div>
  </Show>
);

export const ChannelIdentity: Component<{
  channel: IndexedChannel | undefined;
  handle: string;
}> = (props) => (
  <div class="flex items-center gap-3 transition-all duration-500 ease-out">
    <ChannelAvatar channel={props.channel} />
    <div>
      <p class="text-lg font-semibold text-white">{props.channel?.name ?? props.handle}</p>
      <p class="text-sm text-vesper-muted">
        {props.channel?.handle ?? props.handle}
        <Show when={props.channel}> · {props.channel?.indexed_video_count} videos indexed</Show>
      </p>
    </div>
  </div>
);

const ChannelAvatar: Component<{ channel: IndexedChannel | undefined }> = (props) => (
  <Show
    when={props.channel?.avatar_url}
    fallback={
      <div class="grid size-11 place-items-center border border-vesper-border bg-vesper-panel2 text-sm font-semibold text-vesper-yellow">
        {(props.channel?.handle ?? "?").slice(1, 3).toUpperCase()}
      </div>
    }
  >
    {(avatarUrl) => (
      <img
        src={avatarUrl()}
        alt=""
        class="size-11 border border-vesper-border object-cover"
        loading="eager"
      />
    )}
  </Show>
);
