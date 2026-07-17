import { For, Show } from "solid-js";
import type { Component } from "solid-js";
import { highlightedParts } from "../utils";

export const HighlightedText: Component<{
  text: string;
  highlight?: string;
  query: string;
}> = (props) => {
  const parts = () => highlightedParts(props.text, props.highlight, props.query);

  return (
    <>
      <For each={parts()}>
        {(part) => (
          <Show when={part.highlighted} fallback={<>{part.text}</>}>
            <mark class="bg-vesper-yellow/20 px-1 font-semibold text-vesper-yellow">
              {part.text}
            </mark>
          </Show>
        )}
      </For>
    </>
  );
};
