#!/usr/bin/env node
/**
 * Controlla che tutte le lingue dell'interfaccia abbiano le stesse chiavi e
 * gli stessi segnaposto (`{{name}}`). Il fallback a runtime nasconde le
 * traduzioni mancanti: qui invece il problema diventa un errore esplicito.
 *
 * Uso: `pnpm i18n:check`
 */
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const localesDir = join(here, "..", "src", "i18n");
const reference = "it";
const locales = ["it", "en", "es"];

/** Appiattisce `{ a: { b: "x" } }` in `{ "a.b": "x" }`. */
function flatten(value, prefix = "", out = {}) {
  for (const [key, child] of Object.entries(value)) {
    const path = prefix ? `${prefix}.${key}` : key;
    if (child && typeof child === "object" && !Array.isArray(child)) {
      flatten(child, path, out);
    } else {
      out[path] = child;
    }
  }
  return out;
}

function placeholders(value) {
  return [...String(value).matchAll(/\{\{\s*([\w.]+)\s*\}\}/g)]
    .map((match) => match[1])
    .sort();
}

const catalogs = Object.fromEntries(
  locales.map((locale) => [
    locale,
    flatten(JSON.parse(readFileSync(join(localesDir, `${locale}.json`), "utf8"))),
  ]),
);

const referenceKeys = Object.keys(catalogs[reference]).sort();
const problems = [];

for (const locale of locales) {
  if (locale === reference) continue;
  const keys = Object.keys(catalogs[locale]).sort();

  for (const key of referenceKeys) {
    if (!(key in catalogs[locale])) {
      problems.push(`${locale}: chiave mancante "${key}"`);
      continue;
    }
    if (String(catalogs[locale][key]).trim() === "") {
      problems.push(`${locale}: traduzione vuota per "${key}"`);
    }
    const expected = placeholders(catalogs[reference][key]);
    const actual = placeholders(catalogs[locale][key]);
    if (expected.join(",") !== actual.join(",")) {
      problems.push(
        `${locale}: segnaposto diversi per "${key}" (attesi: ${expected.join(", ") || "nessuno"}; trovati: ${actual.join(", ") || "nessuno"})`,
      );
    }
  }

  for (const key of keys) {
    if (!(key in catalogs[reference])) {
      problems.push(`${locale}: chiave in più "${key}"`);
    }
  }
}

if (problems.length > 0) {
  console.error(`i18n: ${problems.length} problema/i\n`);
  for (const problem of problems) console.error(`  - ${problem}`);
  process.exit(1);
}

console.log(
  `i18n: ok · ${referenceKeys.length} chiavi allineate in ${locales.length} lingue (${locales.join(", ")})`,
);
