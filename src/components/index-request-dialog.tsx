import type { Component } from "solid-js";

export const IndexRequestDialog: Component<{
  handle: string;
  busy: boolean;
  error?: string;
  onCancel: () => void;
  onConfirm: () => void;
}> = (props) => (
  <div class="fixed inset-0 z-50 grid place-items-center bg-black/70 p-4" role="presentation">
    <section
      class="w-full max-w-md border border-vesper-border bg-vesper-panel p-5 shadow-2xl"
      role="dialog"
      aria-modal="true"
      aria-labelledby="index-request-title"
    >
      <p id="index-request-title" class="text-lg font-semibold text-white">
        Request indexing
      </p>
      <p class="mt-2 text-sm text-vesper-muted">
        Submit <span class="font-medium text-white">{props.handle}</span> for indexing approval?
      </p>
      <p class="mt-1 text-sm text-vesper-dim">
        An administrator will review this request before indexing begins. Only public captions can
        be indexed.
      </p>
      {props.error ? <p class="mt-3 text-sm text-vesper-red">{props.error}</p> : null}
      <div class="mt-5 flex justify-end gap-2">
        <button
          type="button"
          onClick={props.onCancel}
          disabled={props.busy}
          class="h-10 border border-vesper-border px-4 text-sm text-vesper-muted transition hover:border-white hover:text-white disabled:opacity-60"
        >
          Cancel
        </button>
        <button
          type="button"
          onClick={props.onConfirm}
          disabled={props.busy}
          class="h-10 border border-vesper-green bg-vesper-green px-4 text-sm font-semibold text-vesper-bg transition hover:bg-vesper-blue disabled:opacity-60"
        >
          {props.busy ? "Submitting..." : "Submit request"}
        </button>
      </div>
    </section>
  </div>
);
