import { fireEvent, render, screen } from "@solidjs/testing-library";
import { expect, test, vi } from "vitest";
import { IndexRequestDialog } from "./index-request-dialog";

test("opens as a named modal, handles Escape, and restores focus", async () => {
  const trigger = document.createElement("button");
  trigger.textContent = "Request index";
  document.body.append(trigger);
  trigger.focus();
  const onCancel = vi.fn();

  const { unmount } = render(() => (
    <IndexRequestDialog
      handle="@t3dotgg"
      busy={false}
      onCancel={onCancel}
      onConfirm={() => undefined}
    />
  ));

  const dialog = screen.getByRole("dialog", { name: "Request indexing" });
  expect(dialog).toHaveAttribute("open");
  fireEvent(dialog, new Event("cancel", { cancelable: true }));
  expect(onCancel).toHaveBeenCalledOnce();

  unmount();
  expect(trigger).toHaveFocus();
  trigger.remove();
});

test("announces a recoverable submission error", () => {
  render(() => (
    <IndexRequestDialog
      handle="@t3dotgg"
      busy={false}
      error="HTTP 503"
      onCancel={() => undefined}
      onConfirm={() => undefined}
    />
  ));

  expect(screen.getByRole("alert")).toHaveTextContent("Unable to submit this request. Try again.");
});
