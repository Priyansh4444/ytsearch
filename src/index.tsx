/* @refresh reload */
import { render } from "solid-js/web";
import "solid-devtools";

import App from "./app";
import { Dashboard } from "./components/dashboard";
import "./index.css";

const root = document.getElementById("root");

if (import.meta.env.DEV && !(root instanceof HTMLElement)) {
  throw new Error(
    "Root element not found. Did you forget to add it to your index.html? Or maybe the id attribute got misspelled?",
  );
}

render(() => (window.location.pathname === "/dashboard" ? <Dashboard /> : <App />), root!);
