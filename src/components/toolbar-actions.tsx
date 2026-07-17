import { Check, ChevronLeft, Copy } from "lucide-solid";
import type { Component } from "solid-js";

export const BackButton: Component<{ onClick: () => void }> = (props) => (
  <button
    type="button"
    onClick={props.onClick}
    class="inline-flex size-10 items-center justify-center border border-vesper-border bg-vesper-panel text-vesper-muted transition hover:border-vesper-yellow hover:text-white"
    aria-label="Go back"
    title="Go back"
  >
    <ChevronLeft size={18} aria-hidden="true" />
  </button>
);

export const CopyLinkButton: Component<{ copied: boolean; onClick: () => void; label?: string }> = (
  props,
) => (
  <button
    type="button"
    onClick={props.onClick}
    class="inline-flex size-10 items-center justify-center border border-vesper-border bg-vesper-panel text-vesper-muted transition hover:border-vesper-yellow hover:text-white"
    aria-label={props.label ?? "Copy link"}
    title={props.label ?? "Copy link"}
  >
    {props.copied ? <Check size={16} aria-hidden="true" /> : <Copy size={16} aria-hidden="true" />}
  </button>
);
