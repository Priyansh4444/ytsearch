import { For, Show } from "solid-js";
import type { Component } from "solid-js";
import type { IndexedChannel } from "../types";
import { normalizeHandle, routeForHandle } from "../utils";
import { ChannelAvatar, indexedCountLabel } from "./ChannelProfile";

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
                props.onSelect(channel.handle);
              }}
              class={`group flex max-w-full items-center gap-3 border px-3 py-2 transition duration-200 ${
                selected()
                  ? "border-vesper-yellow bg-vesper-panel2 text-white"
                  : "border-vesper-border text-vesper-muted hover:border-vesper-yellow hover:text-white"
              }`}
            >
              <ChannelAvatar channel={channel} size="compact" />
              <span class="min-w-0 leading-tight">
                <span class="block truncate text-sm font-medium">{channel.name}</span>
                <span class="block truncate text-xs text-vesper-dim">
                  {channel.handle} · {indexedCountLabel(channel)}
                </span>
              </span>
            </a>
          );
        }}
      </For>
    </div>
  </Show>
);
