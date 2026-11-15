import { API_BASE_URL } from "./api";
import { decodeIndexQueueList, decodeIndexQueueRequest, requestWireJson } from "./bootstrap-api";
import type { IndexQueueRequest } from "./types";

export function listIndexRequests(passkey: string): Promise<IndexQueueRequest[]> {
  return requestWireJson(
    `${API_BASE_URL}/api/dashboard/index-requests?limit=1000`,
    "Could not load requests",
    "Requests returned an invalid response",
    decodeIndexQueueList,
    undefined,
    { headers: { Authorization: `Bearer ${passkey}` }, cache: "no-store" },
  );
}

export function approveIndexRequest(id: string, passkey: string): Promise<IndexQueueRequest> {
  return requestWireJson(
    `${API_BASE_URL}/api/dashboard/index-requests/${encodeURIComponent(id)}/approve`,
    "Could not approve request",
    "Approval returned an invalid response",
    (input) => {
      if (typeof input !== "object" || input === null || !("request" in input))
        throw new TypeError("Expected a request");
      return decodeIndexQueueRequest(input.request);
    },
    undefined,
    { method: "POST", headers: { Authorization: `Bearer ${passkey}` }, cache: "no-store" },
  );
}
