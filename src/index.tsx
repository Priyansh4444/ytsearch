/* @refresh reload */
import { Loading, lazy } from "solid-js";
import { render } from "@solidjs/web";

import App from "./app";
import "./index.css";

const root = document.getElementById("root");

if (import.meta.env.DEV && !(root instanceof HTMLElement)) {
  throw new Error(
    "Root element not found. Did you forget to add it to your index.html? Or maybe the id attribute got misspelled?",
  );
}

if (window.location.pathname === "/dashboard") {
  const Dashboard = lazy(() =>
    import("./components/dashboard").then(({ Dashboard: component }) => ({
      default: component,
    })),
  );
  render(
    () => (
      <Loading
        fallback={
          <main class="grid min-h-svh place-items-center bg-vesper-bg text-sm text-vesper-muted">
            Loading dashboard…
          </main>
        }
      >
        <Dashboard />
      </Loading>
    ),
    root!,
  );
} else {
  render(() => <App />, root!);
}
