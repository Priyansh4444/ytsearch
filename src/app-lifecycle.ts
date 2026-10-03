import { onSettled } from "solid-js";
import { initializeAnalytics } from "./analytics";
import { fetchSearchAvailability, indexStatus } from "./api";
import type { IndexedChannel, SearchAvailability } from "./types";

type AppLifecycleOptions = {
  availability: () => SearchAvailability;
  setAvailability: (value: SearchAvailability) => void;
  refreshChannels: (signal?: AbortSignal) => Promise<IndexedChannel[]>;
  setIndexingToast: (value: string | undefined) => void;
  setIndexingChannelPreview: (value: IndexedChannel | undefined) => void;
  syncRoute: () => void;
};

export function useAppLifecycle(options: AppLifecycleOptions): void {
  onSettled(() => {
    initializeAnalytics();
    window.addEventListener("popstate", options.syncRoute);
    let disposed = false;
    let shownRequestId = "";
    let previousStatus = "idle";
    let toastTimeout: number | undefined;
    const controller = new AbortController();

    const refreshAvailability = () => {
      if (document.hidden) return;
      // Keep the last server answer until the next one arrives. The
      // provisional value follows the scheduled window, so resetting to it
      // closed search every morning under always-on mode and unmounted the
      // results, including a playing video.
      void fetchSearchAvailability(controller.signal)
        .then((value) => {
          if (disposed) return;
          options.setAvailability(value);
          if (value.search_enabled)
            void options.refreshChannels(controller.signal).catch(() => undefined);
        })
        .catch(() => {
          if (options.availability().search_enabled)
            void options.refreshChannels(controller.signal).catch(() => undefined);
        });
    };

    const checkStatus = () => {
      if (document.hidden) return;
      void indexStatus(undefined, controller.signal)
        .then((status) => {
          if (disposed) return;
          if (
            options.availability().search_enabled &&
            (status.status !== previousStatus || ["running", "queued"].includes(status.status))
          )
            void options.refreshChannels(controller.signal).catch(() => undefined);
          previousStatus = status.status;
          if (
            !["running", "queued"].includes(status.status) ||
            !status.request_id ||
            status.request_id === shownRequestId
          )
            return;
          shownRequestId = status.request_id;
          options.setIndexingToast(status.handle);
          options.setIndexingChannelPreview(status.channel);
          if (toastTimeout) window.clearTimeout(toastTimeout);
          toastTimeout = window.setTimeout(() => options.setIndexingToast(undefined), 5_000);
        })
        .catch(() => undefined);
    };

    const refreshVisiblePage = () => {
      if (!document.hidden) {
        refreshAvailability();
        checkStatus();
      }
    };

    refreshAvailability();
    checkStatus();
    document.addEventListener("visibilitychange", refreshVisiblePage);
    const statusInterval = window.setInterval(checkStatus, 15_000);
    const availabilityInterval = window.setInterval(refreshAvailability, 60_000);

    return () => {
      disposed = true;
      controller.abort();
      window.removeEventListener("popstate", options.syncRoute);
      document.removeEventListener("visibilitychange", refreshVisiblePage);
      window.clearInterval(statusInterval);
      window.clearInterval(availabilityInterval);
      if (toastTimeout) window.clearTimeout(toastTimeout);
    };
  });
}
