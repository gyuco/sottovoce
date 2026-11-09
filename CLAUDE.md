# Sottovoce

Assistente desktop per meeting (Teams, Meet, Zoom) che **trascrive**, **traduce**, **suggerisce** cosa dire/chiedere e **riassume**. "Sottovoce" = ti sussurra in tempo reale.

Riscrittura in **Tauri 2 (Rust) + React** del prototipo Python in `/Users/giuseppeconcas/workspace/transcriber` (Tkinter + Vosk + CAM++ + Ollama). Il prototipo resta come riferimento per logica di diarization (`diarization.py`), prompt e UX.

Documento di design completo (ricerca, confronti, prompt da riusare): `docs/DESIGN.md`. Leggerlo prima di decisioni architetturali.

## Stato

Passo 1 fatto. Passo 2 (STT cloud end-to-end) implementato, in verifica con chiave Deepgram reale. Task su board `my-planner-Sottovoce` (tag `roadmap-2`).

- Workspace Cargo alla radice: `src-tauri` (app), `crates/sv-audio` (cattura), `crates/sv-stt` (trait `SttEngine`, `FakeEngine`, motore Deepgram), `crates/sv-models` (catalogo modelli locali su HF fissati a commit + SHA-256, download con resume), `crates/sv-vad` (Silero VAD via crate ufficiale `sherpa-onnx`, `Segmenter` per canale → `SpeechSegment` con padding; `max_speech` di Silero è morbido, il taglio rigido è nostro). `crates/sv-stt-local`: `LocalEngine` (`SttEngine`): VAD per canale → Parakeet TDT 0.6B v3 int8 (sherpa-onnx, CPU) su un thread di decodifica condiviso; parziale "…" mentre si parla; lingua it/en stimata dal testo. Whisper turbo scartato (RTF ~2 su CPU). `crates/sv-diar`: diarization su "Loro" (port di `diarization.py`): `CampPlus` (impronta CAM++), `SpeakerTracker` (automatico), `cluster` (k-means sferico a numero fissato, con rietichettatura), `AudioRing` (audio recente per ritagliare la frase da `start`/`end`, vale per ogni motore). I crate non dipendono da Tauri. Il `target/` è alla radice.
- `crates/sv-store`: persistenza SQLite (`rusqlite` bundled, WAL, `user_version`). DB in `app_data_dir/sottovoce.db`; tabelle `meetings`, `segments`, `translations`, `speakers`, `assists`, `compression_blocks`, `summaries`. Il testo di frasi e traduzioni non è nel DB: sta in `app_data_dir/transcripts/<meeting_id>.txt` (una riga per voce) e le tabelle conservano solo il riferimento (file del meeting + numero di riga); la ricerca dello storico legge i file. La migrazione 4 sposta i testi già salvati su file, elimina le colonne `text` e fa VACUUM per non lasciarne traccia nel DB. `Writer` scrive da un thread dedicato a blocchi in transazione ogni ~1 s (mai sul pump STT); `Store` legge e, all'avvio, chiude i meeting rimasti aperti dopo un crash usando l'ultimo evento. Nessun audio salvato. UI: vista **Storico** (lista paginata, ricerca, dettaglio in sola lettura, rinomina/elimina, export Markdown) con comandi `list_meetings`, `get_meeting`, `rename_meeting`, `delete_meeting`, `export_meeting_markdown` (il testo lo compone il frontend in `src/lib/markdown.ts` con le etichette tradotte; il file lo sceglie l'utente dal dialogo di salvataggio aperto lato Rust in `export.rs`). Compressione rolling ogni 5 min (`compression.rs`: summary/topics/important_moments, usata dal live assist e salvata) e riepilogo finale in 3 estrazioni parallele (`summary.rs`: overview/key points/action items), automatico allo stop (`autoSummary`) o rigenerabile dallo storico (`generate_summary`, `update_summary_checklist`). Pannello riepilogo progressivo, marcatori dei momenti nella timeline e checklist spuntabile in UI.
- Ricerca semantica nello storico: `crates/sv-embed` (EmbeddingGemma 2 di Google, solo testo, q4, modello `embeddinggemma-2-q4` nel catalogo; `ort` =2.0.0-rc.13 con `alternative-backend` usa l'ONNX Runtime già collegato da `sherpa-onnx` tramite `OrtGetApiBase`, nessuna seconda copia; il grafo vuole anche `image/video/audio_features` vuoti [0, 512]; uscita già con pooling e normalizzata, troncata a 512d) e `crates/sv-search` (blocchi da 40 parole o 3 frasi con 1 di sovrapposizione + riepilogo/compressione/action item, vettori int8 in `app_data_dir/index/<meeting_id>.vec` **mai nel DB**, impronta del contenuto nell'header per sapere quando rifare l'indice, ricerca esaustiva in memoria, fusione RRF con la ricerca per parole). App: `search.rs`, thread `sv-index` (QoS utility, query con precedenza tra lotti da 8, modello scaricato dopo 5 min di inattività); indicizza allo stop, dopo il riepilogo, alla rinomina e in backfill all'avvio; `delete_meeting` cancella anche il `.vec`. Comandi `search_meetings`, `semantic_index_status`, `reindex_meetings`, evento `search:status`, setting `semanticSearch`. I punteggi del modello sono compressi (circa 0.57–0.75): niente soglia assoluta, margine di 0.05 dal migliore. UI: risultati con passaggi nello Storico (clic → scorre alla frase), card "Ricerca nello storico" nelle Impostazioni.
- Metriche senza LLM: `src/lib/metrics.ts` (`computeMetrics`) dalle frasi finali: tempo di parola Io/Loro, parole/min (dopo 15 s di parlato), domande (gruppi di `?`), monologo più lungo (turno senza interruzioni dell'altro canale). Componente `ConversationMetrics` in `TalkRatio.tsx`, nella sessione live e nel dettaglio dello Storico.
- Nudge senza LLM: `src/lib/nudges.ts` (`evaluateNudge`, regole pure su frasi finali e ms di sessione) + `stores/nudges.ts` (valutazione ogni 5 s nella finestra principale, uno a schermo per 30 s). Regole in ordine di priorità: monologo "Io" ≥ 90 s, quota "Io" > 65% negli ultimi 5 min (> 75% = medium), > 175 parole/min negli ultimi 2 min, "Loro" parla da ≥ 3 min, nessuna domanda dopo 5 min (una volta). Niente nel primo minuto, cooldown 2 min globale e 5 min per tipo. Setting `nudges` (Impostazioni → Durante la call). `NudgeCard` in Live e nel widget.
- Widget overlay (`widget.rs`, `src/widget/Widget.tsx`): seconda finestra `widget` trasparente, senza decorazioni, `always_on_top`, livello `NSStatusWindowLevel` + `FullScreenAuxiliary` per stare sopra le call a schermo intero, `content_protected` (esclusa dallo screen sharing). Posizione/dimensione via `tauri-plugin-window-state` (solo widget). Apertura: pulsante nell'header, voce del tray, scorciatoia globale ⌥⌘S (`tauri-plugin-global-shortcut`, registrata in `setup`: se occupata l'app parte comunque). Nessuna sessione propria: la finestra principale invia uno snapshot (`src/lib/widget.ts`, eventi `widget:state`/`widget:ready`/`widget:action`, throttle 200 ms) con ultime 3 frasi di "Loro" + traduzione, 3 suggerimenti non usati, nudge, cronometro; il widget rimanda stop, apri finestra, chiudi nudge, suggerimento usato. Chiudere la finestra principale chiude il widget. Stesso bundle: `main.tsx` sceglie il componente dall'etichetta della finestra. Richiede `macOSPrivateApi` (trasparenza).
- Traduzione (`translate.rs`): "Loro" → lingua dell'utente; "Io" → lingua del meeting (setting `translateMine`, default attivo, vale con `autoTranslate`). La lingua del meeting è `theirLanguage` se fissa, altrimenti la più frequente tra le ultime 6 rilevate sulle frasi di "Loro" (`MeetingLanguage`, aggiornata nel pump); finché è ignota "Io" non si traduce. Frasi già nella lingua di arrivo saltate. `TranslationEvent.language` = lingua della traduzione (salvata così nel DB). Pulsante "Traduci" su entrambi i canali (`translate_segment` con `channel`).
- Audio di sistema: Core Audio process tap in `crates/sv-audio/src/system/macos_tap.rs`.
- STT: `crates/sv-stt/src/deepgram/` (WebSocket Nova-3, `language=multi`). Le porzioni `is_final` sono unite in frasi: `Final` su `speech_final`/`UtteranceEnd`.
- Codice locale: `codebase.rs` + `inspect_code_path`. Nel pannello **Contesto meeting** si può indicare una cartella di progetto (selettore cartelle via `tauri-plugin-dialog`, campo `MeetingContext.codePath`, non persistito). La UI la verifica con debounce (esiste? quanti file? quali linguaggi?) e il live assist legge in sola lettura un indice dei sorgenti più estratti reali (`file:riga`) cercati dai termini identificatore citati nelle ultime frasi, aggiunti al prompt come blocco `CODEBASE`; il modello deve usarli per verificare nomi/API e non inventare. Limiti: niente `node_modules`/`target`/`.git`, symlink esclusi, max 8k file, 400 ms di scansione, 150 ms/6 MB per ricerca, 1.600 caratteri nel prompt.
- Chat in call: `chat.rs` + comando `ask_model` e pannello `MeetingChat` (visibile solo a sessione attiva). La conversazione vive nel frontend (`stores/chat.ts`, ultimi 8 turni); il backend aggiunge al prompt contesto meeting, ultime ~60 frasi etichettate con i nomi dei parlanti, e blocco `CODEBASE` con estratti cercati dai termini della domanda + trascrizione. Una chiamata non-streaming (timeout 45 s, max 700 token) col modello di `insight_llm` (assist, altrimenti traduzione); `localOnly` vale anche qui. Stato azzerato a ogni `start_session`.
- App: `session.rs` (bus audio per sessione → uno stream STT per canale → `ipc::Channel<SttEvent>`), `diarize.rs` (task con buffer audio di "Loro": il `Final` aspetta il parlante, max 2 s; rietichettature come `SttEventKind::Speakers`; nomi dei parlanti in `MeetingContext.speakers`, usati nei prompt), `stt.rs` (factory del motore dai settings `sttEngine`/`localSttModel`; `localOnly` forza STT locale e blocca l'avvio se l'LLM usato non è Ollama/localhost), `secrets.rs` (API key nel Keychain, mai al frontend), `settings.rs` (`settings.json` in app config dir, patch parziali).
- Modelli locali: app data dir `models/<id>/`, marker `.verified.json` (dimensione + mtime) per lo stato rapido; comandi `list/download/cancel/delete/verify_local_model`, UI in Impostazioni → Modelli locali. `sherpa-onnx` al primo build scarica le librerie statiche dalle release GitHub di k2-fsa (cache in `target/sherpa-onnx-prebuilt`).
- Prova da terminale: `cargo run -p sv-models --example fetch -- [id]` (senza id elenca lo stato); `cargo run -p sv-vad --example segments -- 30 [--system]`; `cargo run --release -p sv-stt-local --example live -- 30 [--system]`; `cargo run --release -p sv-diar --example speakers -- <wav 16k mono> [--speakers N]`; `cargo run --release -p sv-stt-local --example bench -- parakeet <cartella wav+txt> [--noise 10]`; `cargo run -p sv-audio --example levels -- 5`; `cargo run --release -p sv-embed --example embed -- "query" ["documento" ...]`; `cargo run --release -p sv-search --example search -- <cartella dati con sottovoce.db e transcripts/> "query" ...` (indice in `<cartella>/index`); `DEEPGRAM_API_KEY=... cargo run -p sv-stt --example live -- 30 [--system]`; `[LLM_API_KEY=...] cargo run -p sv-llm --example chat -- [base_url] [model] [--json]` (default Ollama locale). Il tap da terminale resta muto se il terminale non ha il permesso "audio di sistema": per verificare "Loro" usare il bundle (`pnpm app:debug`).
- Latenza STT: a fine sessione log `riepilogo latenza STT` per canale (primo parziale, p50/p95/max di parziali e final, in ms). I log si vedono lanciando il binario da terminale: `target/debug/bundle/macos/Sottovoce.app/Contents/MacOS/sottovoce`.
- Comandi: `pnpm tauri dev`, `pnpm app:debug`, `pnpm typecheck`, `pnpm lint`, `cargo clippy --workspace --all-targets`, `cargo fmt --all --check`, `cargo test --workspace`.

## Rilascio (CI)

- `.github/workflows/ci.yml`: su PR verso `main`/`develop` esegue typecheck, lint, i18n, `cargo fmt --check`, clippy (`-D warnings`) e test su runner macOS.
- `.github/workflows/release.yml`: a ogni merge su `main` compila `Sottovoce.app` + `.dmg` e crea una **release draft** (tag `v<versione>-<sha7>`, versione letta da `tauri.conf.json`). La release non è pubblica finché non la pubblichi a mano; gli asset di un re-run sullo stesso commit vengono sostituiti.
- Firma: senza `APPLE_CERTIFICATE`/`APPLE_SIGNING_IDENTITY` (e `APPLE_ID`/`APPLE_PASSWORD`/`APPLE_TEAM_ID` per la notarizzazione) la build è ad-hoc; su un altro Mac serve `xattr -dr com.apple.quarantine Sottovoce.app`.
- Solo arm64 (`macos-latest`): per Mac Intel serve `macos-13` o un build universal.
- Aggiornamenti: la CI compila con `SOTTOVOCE_BUILD_SHA`, che `update.rs` incorpora; all'avvio l'app chiede all'API GitHub l'ultima release **pubblicata** (`v<versione>-<sha7>`) e, se lo sha è diverso, mostra `UpdateBanner` in alto con "Scarica" (apre la release) e "Ignora" (sha salvato in `localStorage`). Le release in bozza non vengono viste: vanno pubblicate. Controllo al massimo ogni 6 ore; le build locali (senza lo sha) non controllano.

## Principi

- Rust fa il lavoro pesante (audio, VAD, STT, LLM, storage) e custodisce i segreti. React = solo UI.
- API key nel Keychain (crate `keyring`), mai nel frontend né in file in chiaro.
- Provider intercambiabili dietro trait: `SttEngine` (locale/cloud), `LlmProvider` (OpenAI-compatible).
- Modalità "solo locale" sempre disponibile (privacy meeting aziendali / GDPR).
- Due canali audio separati: **mic = "Io"**, **audio di sistema = "Loro"**. Diarization (CAM++) solo sul canale "Loro".
- La traduzione è il differenziatore rispetto a call.md: trattarla come feature primaria.

## Stack

| Strato | Scelta |
|---|---|
| Cattura audio | `cpal` (mic). Sistema: macOS ScreenCaptureKit (crate `screencapturekit`) o Core Audio process tap (macOS 14.4+), niente BlackHole. Windows: WASAPI loopback via `cpal`. |
| VAD + STT locale | `sherpa-rs` / bindings `sherpa-onnx` (Silero VAD, Parakeet, Whisper, CAM++) oppure `whisper-rs` (whisper.cpp + Metal). |
| STT cloud | WebSocket streaming (Deepgram / Soniox / AssemblyAI / ElevenLabs) — scegliere con bake-off. |
| LLM | Client OpenAI-compatible con `reqwest` + `tokio`. Modello configurabile per task. |
| Storage | SQLite (`sqlx` o `rusqlite`). |
| IPC | `tauri::ipc::Channel` per stream trascrizione; `invoke` per comandi. |
| Frontend | React + Vite + TypeScript + Tailwind + shadcn/ui + Zustand. |
| Finestre | Main (storico, riepiloghi, settings) + widget overlay `always_on_top`. |

## Pipeline

```
mic ─┐                       ┌─> traduzione per frase (LLM veloce)
     ├─> VAD ─> STT ─> bus ──┼─> live assist ogni 20s (JSON say_this/ask_this)
sys ─┘   (per canale)        ├─> compressione contesto ogni 5 min
                             └─> SQLite + Channel → UI
```

Un task tokio per stadio; canali `tokio::sync::mpsc`/`broadcast` tra stadi.

## Roadmap

1. Scaffold Tauri 2 + React/Vite/TS/Tailwind; cattura mic + audio di sistema; meter livello ingresso per canale in UI.
2. STT cloud streaming via WebSocket (pipeline end-to-end funzionante).
3. `LlmProvider` OpenAI-compatible → traduzione per frase + live assist JSON con dedup + campo "obiettivo meeting".
4. STT locale (VAD + Parakeet/Whisper turbo) come secondo `SttEngine`; CAM++ sul canale "Loro".
5. SQLite (meeting, segmenti) + compressione 5 min + riepilogo finale (3 prompt) + export Markdown.
6. Metriche (talk ratio, WPM, domande) + nudge rate-limited + widget overlay.
7. Dopo: tool MCP, integrazione calendario.

## Convenzioni

- Lingua UI: italiano di default, con inglese e spagnolo; cataloghi in `src/i18n/` (`it`, `en`, `es`). La lingua si sceglie in Impostazioni → Interfaccia e viene ricordata.
- **Regola: ogni testo dell'interfaccia va tradotto in tutte e 3 le lingue (italiano, inglese, spagnolo) nello stesso change.** Mai una chiave solo in una o due lingue, mai stringhe UI hardcoded nei componenti. Dopo aver toccato i testi `pnpm i18n:check` deve passare (chiavi e segnaposto allineati).
- Commit Conventional Commits.
- Rust: `cargo clippy` e `cargo fmt` puliti; frontend: `tsc --noEmit` + eslint.
