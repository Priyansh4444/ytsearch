import { createEffect, createSignal, type Accessor } from "solid-js";
import { searchTranscripts } from "./api";
import type { SearchRequest, SearchResponse } from "./types";

export type SearchResourceState =
  | { status: "idle" }
  | { status: "loading" }
  | { status: "success"; response: SearchResponse }
  | { status: "error"; error: unknown };

export function createSearchResource(
  request: Accessor<SearchRequest | undefined>,
): Accessor<SearchResourceState> {
  const [state, setState] = createSignal<SearchResourceState>({ status: "idle" });
  let currentController: AbortController | undefined;

  createEffect(request, (nextRequest) => {
    if (!nextRequest) {
      currentController = undefined;
      setState({ status: "idle" });
      return;
    }

    const controller = new AbortController();
    currentController = controller;
    setState({ status: "loading" });

    void searchTranscripts(nextRequest, controller.signal).then(
      (response) => {
        if (currentController === controller) setState({ status: "success", response });
      },
      (error: unknown) => {
        if (currentController === controller && !controller.signal.aborted) {
          setState({ status: "error", error });
        }
      },
    );

    return () => {
      if (currentController === controller) currentController = undefined;
      controller.abort();
    };
  });

  return state;
}
