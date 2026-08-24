import { onCleanup, onSettled } from "solid-js";
import type { Component } from "solid-js";

export const IndexRequestDialog: Component<{
  handle: string;
  busy: boolean;
  error?: string;
  onCancel: () => void;
  onConfirm: () => void;
}> = (props) => {
  let dialog: HTMLDialogElement | undefined;
  const previouslyFocused =
    document.activeElement instanceof HTMLElement ? document.activeElement : null;

  onSettled(() => {
    if (typeof dialog?.showModal === "function") dialog.showModal();
    else dialog?.setAttribute("open", "");
  });

  onCleanup(() => {
    if (dialog?.open && typeof dialog.close === "function") dialog.close();
    previouslyFocused?.focus();
  });

  return (
    <dialog
      ref={(element) => (dialog = element)}
      class="m-auto w-[calc(100%-2rem)] max-w-md border border-vesper-control bg-vesper-panel p-5 text-vesper-text shadow-2xl backdrop:bg-black/70"
      aria-labelledby="index-request-title"
      aria-describedby="index-request-description"
      onCancel={(event) => {
        event.preventDefault();
        if (!props.busy) props.onCancel();
      }}
    >
      <p id="index-request-title" class="text-lg font-semibold text-white">
        Request indexing
      </p>
      <p id="index-request-description" class="mt-2 text-sm text-vesper-muted">
        Submit <span class="font-medium text-white">{props.handle}</span> for indexing approval?
      </p>
      <p class="mt-1 text-sm text-vesper-dim">
        An administrator will review this request before indexing begins. Only public captions can
        be indexed.
      </p>
      {props.error ? (
        <p id="index-request-error" class="mt-3 text-sm text-vesper-red" role="alert">
          Unable to submit this request. Try again.
        </p>
      ) : null}
      <div class="mt-5 flex justify-end gap-2">
        <button
          type="button"
          onClick={props.onCancel}
          disabled={props.busy}
          class="pressable h-10 border border-vesper-control px-4 text-sm text-vesper-muted hover:border-white hover:text-white disabled:opacity-60"
        >
          Cancel
        </button>
        <button
          type="button"
          onClick={props.onConfirm}
          disabled={props.busy}
          aria-describedby={props.error ? "index-request-error" : undefined}
          class="pressable h-10 border border-vesper-green bg-vesper-green px-4 text-sm font-semibold text-vesper-bg hover:bg-vesper-blue disabled:opacity-60"
        >
          {props.busy ? "Submitting…" : "Submit request"}
        </button>
      </div>
    </dialog>
  );
};
