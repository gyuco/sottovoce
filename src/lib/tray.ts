import { invoke, isTauri } from "@tauri-apps/api/core";
import { Image } from "@tauri-apps/api/image";
import { Menu, type MenuOptions } from "@tauri-apps/api/menu";
import { TrayIcon } from "@tauri-apps/api/tray";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type { TFunction } from "i18next";

import trayIcon from "@/assets/tray-icon.png?inline";

/**
 * Icona di Sottovoce nel menu bar (macOS) / area di notifica (Windows, Linux):
 * clic = torna alla finestra, clic destro = voci "Mostra", "Widget" ed "Esci".
 */
let tray: TrayIcon | null = null;
let building: Promise<void> | null = null;
/** Lingua con cui è stato costruito il menu attuale. */
let menuLanguage: string | null = null;

/** I PNG arrivano da Vite come data URL (`?inline`): qui tornano byte. */
function pngBytes(dataUrl: string) {
  const binary = atob(dataUrl.slice(dataUrl.indexOf(",") + 1));
  return Uint8Array.from(binary, (char) => char.charCodeAt(0));
}

/** Riporta in primo piano la finestra principale, anche se ridotta a icona. */
export async function showMainWindow() {
  const window = getCurrentWindow();
  if (await window.isMinimized()) await window.unminimize();
  await window.show();
  await window.setFocus();
}

function menuOptions(t: TFunction): MenuOptions {
  return {
    items: [
      { id: "show", text: t("tray.show"), action: () => void showMainWindow() },
      { id: "widget", text: t("tray.widget"), action: () => void invoke("toggle_widget") },
      { item: "Separator" },
      { id: "quit", text: t("tray.quit"), action: () => void invoke("quit_app") },
    ],
  };
}

async function create(t: TFunction, language: string) {
  tray = await TrayIcon.new({
    id: "sottovoce",
    // Su macOS l'immagine è "template": il sistema la colora per la menu bar.
    icon: await Image.fromBytes(pngBytes(trayIcon)),
    iconAsTemplate: true,
    tooltip: t("tray.tooltip"),
    menu: await Menu.new(menuOptions(t)),
    showMenuOnLeftClick: false,
    action: (event) => {
      if (event.type === "Click" && event.button === "Left" && event.buttonState === "Up") {
        void showMainWindow();
      }
    },
  });
  menuLanguage = language;
}

/**
 * Crea l'icona nel menu bar una sola volta e ne ritraduce le voci quando cambia
 * la lingua dell'interfaccia. Fuori da Tauri (browser) non fa nulla.
 */
export async function syncTray(t: TFunction, language: string) {
  if (!isTauri()) return;

  if (tray) {
    if (menuLanguage === language) return;
    await tray.setMenu(await Menu.new(menuOptions(t)));
    await tray.setTooltip(t("tray.tooltip"));
    menuLanguage = language;
    return;
  }

  // StrictMode/esecuzioni doppie: una sola creazione alla volta.
  if (building) return;
  building = create(t, language)
    .catch((error: unknown) => console.error("icona del menu bar non creata:", error))
    .finally(() => {
      building = null;
    });
  await building;
}
