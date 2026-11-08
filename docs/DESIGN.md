# Sottovoce — Design e ricerca

Raccolta delle decisioni e della ricerca fatta il 2026-11-07, partendo dal prototipo Python `/Users/giuseppeconcas/workspace/transcriber` e dall'analisi del repo [video-db/call.md](https://github.com/video-db/call.md).

## 1. Prototipo di partenza (transcriber, Python)

- Tkinter GUI, cattura audio via `sounddevice` (16 kHz mono int16), BlackHole + Multi-Output Device per l'audio di sistema.
- STT: Vosk small (`it`, `en`, `es`) con parziali.
- Diarization: CAM++ (3D-Speaker, `3dspeaker_campplus_sv_zh_en_16k-common_advanced.onnx`) via `sherpa-onnx`; `SpeakerTracker` con similarità coseno, `cluster_vectors` (k-means sferico) quando il numero di speaker è fissato.
- LLM: Ollama (`llama3.2:3b`) per traduzione + risposta suggerita per ogni frase (parsing regex `TRANSLATION:/REPLY:`), riepilogo on-demand.
- Anti-eco: frasi identiche entro 20s ignorate.
- Limiti: Vosk poco accurato, nessuna persistenza, setup BlackHole complesso, un solo canale (diarization su tutti), contesto LLM non scala su call lunghe.

## 2. Cosa prendere da call.md

call.md = Electron + React + TS, audio e LLM nel cloud (VideoDB). Non tradurre lo stack: prendere le idee.

1. **Dual-channel**: mic ("Io") e audio di sistema ("Loro") trascritti separatamente → "Io vs Loro" gratis, suggerimenti rivolti solo all'utente.
2. **Live assist periodico** (ogni 20s, separato dalla traduzione): output JSON `{"say_this": [...], "ask_this": [...]}`, 0–3 item ciascuno, array vuoti se non serve. Dedup dei suggerimenti già mostrati.
3. **Contesto meeting** a inizio sessione (nome, obiettivo, note, checklist) iniettato in tutti i prompt.
4. **Compressione rolling** ogni 5 min: `summary`, `topics`, `important_moments` (objection/commitment/question/pain_point/decision). Contesto = storico compresso + ultime ~20 frasi. Budget ~8000 token.
5. **Riepilogo finale in 3 estrazioni parallele**: overview narrativa, key points per topic attribuiti, action items.
6. **Persistenza** SQLite + export Markdown.
7. **Metriche senza LLM**: talk ratio, WPM, domande, monologhi; nudge con cooldown 2 min (es. talk ratio "me" > 0.65 → low, > 0.75 → medium, dopo 60s di call).
8. Dopo: widget flottante, webhook fine meeting (n8n/Zapier), MCP auto-triggering, calendario.

call.md **non traduce**: è il nostro differenziatore.

## 3. Provider LLM

Livello unico **OpenAI-compatible** (`base_url` + `api_key` + `model`):

- **Ollama** `http://localhost:11434/v1` — modalità privacy.
- **OpenCode Zen** `https://opencode.ai/zen/v1`, chiave API (`OPENCODE_API_KEY`), pay-per-use. Attenzione: modelli GPT usano `/v1/responses`, Claude `/v1/messages` (stile Anthropic), Gemini client Google; gli altri chat completions. Catalogo: `GET https://opencode.ai/zen/v1/models`. Prezzi: https://opencode.ai/docs/zen
- **OpenRouter, Groq, Anthropic, OpenAI**: stesso schema (Anthropic eventualmente con endpoint nativo `/v1/messages`).

Da NON fare:
- `opencode serve` = agente di coding, non un'API LLM.
- Riutilizzare token OAuth di abbonamenti ChatGPT/Claude Pro in un'app propria: viola i ToS. Usare chiavi API.

Routing per task (configurabile):
| Task | Modello |
|---|---|
| Traduzione per frase | piccolo, bassa latenza (Groq / "mini") |
| Live assist 20s | medio, output JSON |
| Compressione 5 min | piccolo/medio, JSON |
| Riepilogo finale | il migliore disponibile (una sola chiamata) |

Privacy: con provider cloud il contenuto esce dalla macchina → toggle "solo locale" esplicito.

## 4. Strategia trascrizione

Trait `SttEngine` con due implementazioni.

| | Locale (VAD + Parakeet / Whisper large-v3-turbo) | Cloud streaming |
|---|---|---|
| Qualità | buona su Apple Silicon | migliore, soprattutto audio rumoroso |
| Latenza | 1–3s per frase (non vero streaming) | ~300ms, parziali live |
| Costo | 0 | es. Deepgram Nova-3 multilingual ≈ $0.0058/min ≈ $0.35/h per canale (×2 canali) |
| Privacy | totale | audio a terzi |
| Complessità | packaging modelli/ONNX in Rust | WebSocket |
| Extra | — | diarization inclusa; Soniox dichiara traduzione live 60+ lingue |

Candidati cloud: **Deepgram** (Nova-3 / Flux, latenza parziali), **AssemblyAI** Universal-Streaming (~300ms P50, niente traduzione), **Soniox** (claim WER italiano 1.25%, traduzione integrata — claim del vendor), **ElevenLabs Scribe v2 Realtime**.

Decisione:
- Default **locale** (VAD + Parakeet o Whisper turbo) — coerente con privacy, gratis, offline.
- **Cloud "alta qualità"** opzionale, un solo provider scelto con bake-off: 10 min di call reali (it + en, rumore Teams), misurare WER e latenza primo parziale. Benchmark dei vendor discordanti.
- Se vince Soniox, valutare di usarlo anche per la traduzione (elimina una chiamata LLM per frase).
- Ordine di implementazione: prima cloud (pipeline end-to-end più rapida), poi locale.

## 5. Riferimenti open source

- **call.md** (Electron) — servizi da studiare: `src/main/services/live-assist.service.ts`, `copilot/context-manager.service.ts`, `copilot/summary-generator.service.ts`, `copilot/nudge-engine.service.ts`, `copilot/conversation-metrics.service.ts`.
- **Meetily** e **Hyprnote** — app Tauri per trascrizione meeting: riferimento per cattura audio di sistema su macOS.

## 6. Prompt da riusare (adattati da call.md)

### Live assist (system)

```
You are a live meeting coach. You receive a rolling 20-second transcript from an ongoing meeting. Your job is to surface helpful nudges for the User.

All assistance must be directed to the "User" (labeled [Io]). Do NOT provide assistance for other participants (labeled [Loro] / Speaker N). Focus exclusively on what the User can say or ask.

You may receive MEETING CONTEXT (name, purpose, notes, goals): keep suggestions aligned with it.

You may also receive CODEBASE content: a file outline and real excerpts read from the user's local project. Treat it as ground truth: when a participant mentions a file, function, API or behavior, check it against the excerpts, use exact names, and never invent code, files or APIs. If the excerpts do not cover a point, say so instead of guessing.

Surface: a way to respond to something said; an opportunity to summarize or steer; a topic to park; a decision to capture; a commitment to acknowledge; a clarifying question; a vague claim to pin down; follow-up questions.

Only surface assists when the User would genuinely benefit. Otherwise return empty arrays.

Output JSON only:
{"say_this": ["..."], "ask_this": ["..."]}

Rules: 0-3 items per array; every suggestion must connect to the transcript; ready-to-use first-person lines; write them in the meeting language (and add the translation in the user's language if different).
```

### Compressione 5 minuti

```
Summarize this 5-minute meeting segment in 3-4 bullet points. Extract key information to reference later.
Respond with JSON:
{"summary": "...", "topics": ["..."], "important_moments": [{"time": 305, "type": "objection|commitment|question|pain_point|decision", "text": "..."}]}
```

### Riepilogo finale — overview

```
You are an expert meeting summarizer. Given a meeting transcript with name, description and checklist, produce a short narrative summary.
Single paragraph, no lists. Who was there, what it was about, main threads. Mention participants by name, high-level, no verbatim quotes, no opinions. 3-5 sentences, max 120 words, past tense, third person. Respond only with the paragraph.
```

### Riepilogo finale — key points

```
Identify 2-5 major topics; under each, 2-5 points attributed to who raised them ("Name said/raised/confirmed..."). One concrete sentence each, factual, past tense. Weave checklist items into topics.
JSON only: {"key_points": [{"topic": "...", "points": ["..."]}]}
```

### Riepilogo finale — action items

```
Extract action items, follow-ups, open questions and tasks to handle AFTER the meeting. Include responsible person and deadlines if mentioned.
Good: "John to send the proposal by Friday". Bad: "Discussed the project".
Order by priority if discernible. 3-10 items, never invent; empty array if none.
JSON only: {"checklist": ["..."]}
```

## 7. Fonti

- https://github.com/video-db/call.md
- https://dev.opencode.ai/docs/zen
- https://docs.docker.com/ai/docker-agent/providers/opencode-zen/
- https://docs.getbifrost.ai/providers/supported-providers/opencode
- https://futureagi.substack.com/p/speech-to-text-apis-in-2026-benchmarks
- https://www.assemblyai.com/blog/best-api-models-for-real-time-speech-recognition-and-transcription
- https://soniox.com/compare-stt/soniox-vs-assemblyai/italian
