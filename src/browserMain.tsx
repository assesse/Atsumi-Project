import { createRoot } from "react-dom/client";
import { BrowserApp } from "./features/browser/BrowserApp";
import "./features/browser/BrowserApp.css";
import "./scrollbars.css";
createRoot(document.getElementById("root")!).render(<BrowserApp/>);
