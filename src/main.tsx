import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import { backend } from "./api/backend";
import { installUiDiagnostics } from "./diagnostics/uiDiagnostics";
import { restoreNativeCheckpoint } from "./api/workConsole";
import {
  BackendThumbnailAdapter,
  browserFixtureThumbnailAdapter,
  ThumbnailClient,
  ThumbnailProvider,
} from "./thumbnail";
import "./styles.css";
import "./atsumi-ink.css";

const root = document.getElementById("root");
if (!root) throw new Error("Atsumi root element is missing");

const backendThumbnailAdapter = backend.runtime === "tauri"
  ? new BackendThumbnailAdapter(backend)
  : null;
const thumbnailClient = new ThumbnailClient(backendThumbnailAdapter ?? browserFixtureThumbnailAdapter);
const stopDiagnostics = installUiDiagnostics(backend.runtime === "tauri");

window.addEventListener("beforeunload", () => {
  stopDiagnostics();
  thumbnailClient.dispose();
  backendThumbnailAdapter?.dispose();
}, { once: true });
if (import.meta.hot) import.meta.hot.dispose(() => {
  stopDiagnostics(); thumbnailClient.dispose(); backendThumbnailAdapter?.dispose();
});

void restoreNativeCheckpoint().then(() => createRoot(root).render(
  <StrictMode>
    <ThumbnailProvider client={thumbnailClient}>
      <App />
    </ThumbnailProvider>
  </StrictMode>,
));
