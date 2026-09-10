import { For } from "solid-js";
import type { Component } from "solid-js";
import { highlightedParts } from "../utils";

export const HighlightedText: Component<{
  text: string;
  highlight?: string;
  query: string;
}> = (props) => {
  // Single tracked read below (`For each`); a memo would only add a node +
  // `===` check for a microsecond regex over a short snippet. The parts are
  // static after mount, so a ternary renders the same DOM with zero nodes.
  const parts = () => highlightedParts(props.text, props.highlight, props.query);

  return (
    <For each={parts()}>
      {(part) =>
        part.highlighted ? (
          <mark class="bg-vesper-yellow/20 px-1 font-semibold text-vesper-yellow">{part.text}</mark>
        ) : (
          <>{part.text}</>
        )
      }
    </For>
  );
};
