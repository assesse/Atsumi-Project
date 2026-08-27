import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import { backend } from "./api/backend";
import {
  BackendThumbnailAdapter,
  browserFixtureThumbnailAdapter,
  ThumbnailClient,
  ThumbnailProvider,
} from "./thumbnail";
import "./styles.css";

const root = document.getElementById("root");
if (!root) throw new Error("Atsumi root element is missing");

const backendThumbnailAdapter = backend.runtime === "tauri"
  ? new BackendThumbnailAdapter(backend)
  : null;
const thumbnailClient = new ThumbnailClient(backendThumbnailAdapter ?? browserFixtureThumbnailAdapter);

window.addEventListener("beforeunload", () => {
  thumbnailClient.dispose();
  backendThumbnailAdapter?.dispose();
}, { once: true });

createRoot(root).render(
  <StrictMode>
    <ThumbnailProvider client={thumbnailClient}>
      <App />
    </ThumbnailProvider>
  </StrictMode>,
);
