# Sottovoce

Assistente desktop per meeting (Teams, Meet, Zoom) che **trascrive**, **traduce**, **suggerisce** cosa dire/chiedere e **riassume**. "Sottovoce" = ti sussurra in tempo reale.

Riscrittura in **Tauri 2 (Rust) + React** del prototipo Python in `/Users/giuseppeconcas/workspace/transcriber` (Tkinter + Vosk + CAM++ + Ollama). Il prototipo resta come riferimento per logica di diarization (`diarization.py`), prompt e UX.

Documento di design completo (ricerca, confronti, prompt da riusare): `docs/DESIGN.md`. Leggerlo prima di decisioni architetturali.

## Stato

Passo 1 fatto. Passo 2 (STT cloud end-to-end) implementato, in verifica con chiave Deepgram reale. Task su board `my-planner-Sottovoce` (tag `roadmap-2`).

- Workspace Cargo alla radice: `src-tauri` (app), `crates/sv-audio` (cattura), `crates/sv-stt` (trait `SttEngine`, `FakeEngine`, motore Deepgram), `crates/sv-models` (catalogo modelli locali su HF fissati a commit + SHA-256, download con resume), `crates/sv-vad` (Silero VAD via crate ufficiale `sherpa-onnx`, `Segmenter` per canale → `SpeechSegment` con padding; `max_speech` di Silero è morbido, il taglio rigido è nostro). `crates/sv-stt-local`: `LocalEngine` (`SttEngine`): VAD per canale → Parakeet TDT 0.6B v3 int8 (sherpa-onnx, CPU) su un thread di decodifica condiviso; parziale "…" mentre si parla; lingua it/en stimata dal testo. Whisper turbo scartato (RTF ~2 su CPU). `crates/sv-diar`: diarization su "Loro" (port di `diarization.py`): `CampPlus` (impronta CAM++), `SpeakerTracker` (automatico), `cluster` (k-means sferico a numero fissato, con rietichettatura), `AudioRing` (audio recente per ritagliare la frase da `start`/`end`, vale per ogni motore). I crate non dipendono da Tauri. Il `target/` è alla radice.
- Audio di sistema: Core Audio process tap in `crates/sv-audio/src/system/macos_tap.rs`.
- STT: `crates/sv-stt/src/deepgram/` (WebSocket Nova-3, `language=multi`). Le porzioni `is_final` sono unite in frasi: `Final` su `speech_final`/`UtteranceEnd`.
- App: `session.rs` (bus audio per sessione → uno stream STT per canale → `ipc::Channel<SttEvent>`), `diarize.rs` (task con buffer audio di "Loro": il `Final` aspetta il parlante, max 2 s; rietichettature come `SttEventKind::Speakers`; nomi dei parlanti in `MeetingContext.speakers`, usati nei prompt), `stt.rs` (factory del motore dai settings `sttEngine`/`localSttModel`; `localOnly` forza STT locale e blocca l'avvio se l'LLM usato non è Ollama/localhost), `secrets.rs` (API key nel Keychain, mai al frontend), `settings.rs` (`settings.json` in app config dir, patch parziali).
- Modelli locali: app data dir `models/<id>/`, marker `.verified.json` (dimensione + mtime) per lo stato rapido; comandi `list/download/cancel/delete/verify_local_model`, UI in Impostazioni → Modelli locali. `sherpa-onnx` al primo build scarica le librerie statiche dalle release GitHub di k2-fsa (cache in `target/sherpa-onnx-prebuilt`).
- Prova da terminale: `cargo run -p sv-models --example fetch -- [id]` (senza id elenca lo stato); `cargo run -p sv-vad --example segments -- 30 [--system]`; `cargo run --release -p sv-stt-local --example live -- 30 [--system]`; `cargo run --release -p sv-diar --example speakers -- <wav 16k mono> [--speakers N]`; `cargo run --release -p sv-stt-local --example bench -- parakeet <cartella wav+txt> [--noise 10]`; `cargo run -p sv-audio --example levels -- 5`; `DEEPGRAM_API_KEY=... cargo run -p sv-stt --example live -- 30 [--system]`; `[LLM_API_KEY=...] cargo run -p sv-llm --example chat -- [base_url] [model] [--json]` (default Ollama locale). Il tap da terminale resta muto se il terminale non ha il permesso "audio di sistema": per verificare "Loro" usare il bundle (`pnpm app:debug`).
- Latenza STT: a fine sessione log `riepilogo latenza STT` per canale (primo parziale, p50/p95/max di parziali e final, in ms). I log si vedono lanciando il binario da terminale: `target/debug/bundle/macos/Sottovoce.app/Contents/MacOS/sottovoce`.
- Comandi: `pnpm tauri dev`, `pnpm app:debug`, `pnpm typecheck`, `pnpm lint`, `cargo clippy --workspace --all-targets`, `cargo fmt --all --check`, `cargo test --workspace`.

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
7. Dopo: webhook fine meeting (n8n), tool MCP, integrazione calendario.

## Convenzioni

- Lingua UI: italiano di default, con inglese e spagnolo; cataloghi in `src/i18n/` (`it`, `en`, `es`). La lingua si sceglie in Impostazioni → Interfaccia e viene ricordata.
- **Regola: ogni testo dell'interfaccia va tradotto in tutte e 3 le lingue (italiano, inglese, spagnolo) nello stesso change.** Mai una chiave solo in una o due lingue, mai stringhe UI hardcoded nei componenti. Dopo aver toccato i testi `pnpm i18n:check` deve passare (chiavi e segnaposto allineati).
- Commit Conventional Commits.
- Rust: `cargo clippy` e `cargo fmt` puliti; frontend: `tsc --noEmit` + eslint.
