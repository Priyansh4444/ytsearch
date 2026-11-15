import { fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { beforeEach, expect, test, vi } from "vitest";
import { ApiResponseError } from "../api-error";
import type { IndexQueueRequest } from "../types";
import { DashboardApprovals } from "./dashboard-approvals";

const mocks = vi.hoisted(() => ({ list: vi.fn(), approve: vi.fn() }));
vi.mock("../dashboard-api", () => ({
  listIndexRequests: mocks.list,
  approveIndexRequest: mocks.approve,
  ApiResponseError,
}));
const request: IndexQueueRequest = {
  id: "request-1",
  handle: "@new",
  requested_at: 1,
  source_backend: "raw",
  status: "pending",
};
beforeEach(() => {
  mocks.list.mockReset();
  mocks.approve.mockReset();
});
const unlock = async () => {
  fireEvent.input(screen.getByLabelText("Approval passkey"), { target: { value: "test-passkey" } });
  await waitFor(() =>
    expect(screen.getByRole("button", { name: "Unlock approvals" })).not.toBeDisabled(),
  );
  fireEvent.submit(screen.getByLabelText("Approval passkey").closest("form")!);
};

test("validates passkey, approves once, removes the request, and clears passkey on lock", async () => {
  mocks.list.mockResolvedValue([
    request,
    { ...request, id: "queued", handle: "@queued", status: "queued" },
  ]);
  mocks.approve.mockResolvedValue({ ...request, status: "queued" });
  render(() => <DashboardApprovals channels={[]} />);
  expect(mocks.list).not.toHaveBeenCalled();
  await unlock();
  expect(mocks.list).toHaveBeenCalled();
  fireEvent.click(await screen.findByRole("button", { name: "Approve @new" }));
  await waitFor(() =>
    expect(screen.queryByRole("button", { name: "Approve @new" })).not.toBeInTheDocument(),
  );
  expect(mocks.list).toHaveBeenCalledWith("test-passkey");
  expect(mocks.approve).toHaveBeenCalledExactlyOnceWith("request-1", "test-passkey");
  expect(screen.queryByText("@queued")).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Lock approvals" }));
  expect(await screen.findByLabelText("Approval passkey")).toHaveValue("");
});

test("invalid passkeys keep approvals locked", async () => {
  mocks.list.mockRejectedValue(new ApiResponseError("Unauthorized", 401));
  render(() => <DashboardApprovals channels={[]} />);
  await unlock();
  expect(mocks.list).toHaveBeenCalled();
  expect(await screen.findByRole("alert")).toHaveTextContent("Incorrect passkey");
  expect(await screen.findByLabelText("Approval passkey")).toHaveValue("");
  expect(screen.queryByRole("button", { name: "Lock approvals" })).not.toBeInTheDocument();
});

test("does not present already indexed creators for approval", async () => {
  mocks.list.mockResolvedValue([{ ...request, handle: "@EXISTING" }]);
  render(() => (
    <DashboardApprovals
      channels={[
        {
          id: "channel",
          handle: "@existing",
          name: "Existing",
          channel_id: "UC1",
          public_video_count: 1,
          indexed_video_count: 1,
          indexed_chunk_count: 1,
        },
      ]}
    />
  ));
  await unlock();
  expect(mocks.list).toHaveBeenCalled();
  expect(await screen.findByText("No requests awaiting approval.")).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Approve @EXISTING" })).not.toBeInTheDocument();
});

test("locking ignores an in-flight refresh response", async () => {
  mocks.list.mockResolvedValueOnce([request]);
  let finish: (value: IndexQueueRequest[]) => void = () => undefined;
  mocks.list.mockImplementationOnce(
    () =>
      new Promise<IndexQueueRequest[]>((resolve) => {
        finish = resolve;
      }),
  );
  render(() => <DashboardApprovals channels={[]} />);
  await unlock();
  expect(mocks.list).toHaveBeenCalled();
  finish([request]);
  await screen.findByRole("button", { name: "Lock approvals" });
  mocks.list.mockImplementationOnce(
    () =>
      new Promise<IndexQueueRequest[]>((resolve) => {
        finish = resolve;
      }),
  );
  fireEvent.click(screen.getByRole("button", { name: "Refresh requests" }));
  fireEvent.click(screen.getByRole("button", { name: "Lock approvals" }));
  finish([request]);
  await waitFor(() => expect(screen.getByLabelText("Approval passkey")).toBeInTheDocument());
  expect(screen.queryByRole("button", { name: "Approve @new" })).not.toBeInTheDocument();
});
