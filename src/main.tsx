import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import React from "react";
import ReactDOM from "react-dom/client";

import App from "./App";
import "./i18n";
import "./index.css";
import { WIDGET_LABEL } from "./lib/widget";
import { Widget } from "./widget/Widget";

// Stesso bundle per le due finestre: il widget overlay si riconosce dall'etichetta.
const isWidget = isTauri() && getCurrentWindow().label === WIDGET_LABEL;
if (isWidget) document.documentElement.classList.add("widget");

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>{isWidget ? <Widget /> : <App />}</React.StrictMode>,
);
