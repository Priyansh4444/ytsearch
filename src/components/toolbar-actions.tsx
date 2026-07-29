import type { Component } from "solid-js";
import { CheckIcon, ChevronLeftIcon, CopyIcon } from "./icons";

export const BackButton: Component<{ onClick: () => void }> = (props) => (
  <button
    type="button"
    onClick={props.onClick}
    class="pressable inline-flex size-10 items-center justify-center border border-vesper-control bg-vesper-panel text-vesper-muted hover:border-vesper-yellow hover:text-white"
    aria-label="Go back"
    title="Go back"
  >
    <ChevronLeftIcon size={18} aria-hidden="true" />
  </button>
);

export const CopyLinkButton: Component<{ copied: boolean; onClick: () => void; label?: string }> = (
  props,
) => (
  <button
    type="button"
    onClick={props.onClick}
    class="pressable inline-flex size-10 items-center justify-center border border-vesper-control bg-vesper-panel text-vesper-muted hover:border-vesper-yellow hover:text-white"
    aria-label={props.label ?? "Copy link"}
    title={props.label ?? "Copy link"}
  >
    {props.copied ? (
      <CheckIcon size={16} aria-hidden="true" />
    ) : (
      <CopyIcon size={16} aria-hidden="true" />
    )}
  </button>
);
