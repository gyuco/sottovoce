//! Prova da terminale di un endpoint OpenAI-compatible.
//!
//! ```sh
//! cargo run -p sv-llm --example chat -- [base_url] [model] [--json]
//! LLM_API_KEY=... cargo run -p sv-llm --example chat -- https://opencode.ai/inference/openai/v1 glm-5.3
//! ```
//! Default: Ollama locale, primo modello compatibile.

use sv_llm::{ChatRequest, LlmProvider, Message, OpenAiCompatible, parse_json};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let json = args.iter().any(|a| a == "--json");
    let mut pos = args.iter().filter(|a| !a.starts_with("--"));
    let base_url = pos
        .next()
        .map_or("http://localhost:11434/v1", |s| s.as_str());
    let llm = OpenAiCompatible::new(base_url).with_api_key(std::env::var("LLM_API_KEY").ok());

    let models = llm.list_models().await?;
    let usable: Vec<&str> = models
        .iter()
        .filter(|m| m.usable)
        .map(|m| m.id.as_str())
        .collect();
    println!(
        "{} modelli, {} compatibili: {usable:?}",
        models.len(),
        usable.len()
    );
    let model = match pos.next() {
        Some(m) => m.clone(),
        None => usable
            .first()
            .ok_or("nessun modello compatibile")?
            .to_string(),
    };

    let req = if json {
        ChatRequest::new(
            &model,
            vec![
                Message::system(
                    "Rispondi solo con JSON: {\"say_this\": [string], \"ask_this\": [string]}, 1-2 elementi per array.",
                ),
                Message::user("[Loro] We should ship the beta next week, but QA is behind."),
            ],
        )
        .json()
    } else {
        ChatRequest::new(
            &model,
            vec![
                Message::system("Traduci in italiano. Rispondi solo con la traduzione."),
                Message::user("We should ship the beta next week, but QA is behind."),
            ],
        )
    }
    .temperature(0.2);

    let resp = llm.chat(req).await?;
    println!(
        "{model}: {} ms, {} tentativi, usage {:?}\n{}",
        resp.latency.as_millis(),
        resp.attempts,
        resp.usage,
        resp.content
    );
    if json {
        let v: serde_json::Value = parse_json(&resp.content)?;
        println!("JSON ok: {v}");
    }
    Ok(())
}
