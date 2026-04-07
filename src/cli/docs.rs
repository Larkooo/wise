// `wise docs ask "..."` — wraps the public docs.wise.com /_ask-ai SSE.
//
// We've verified the endpoint shape: it returns text/event-stream with three
// event types in the JSON payloads — `messageId`, `sources`, and chunked
// `answer`. We stream `answer` chunks live to stdout (so the agent can show
// progress) and emit a final JSON summary with `messageId` + `sources` to
// stderr (or to stdout in --no-stream mode).

use anyhow::{Context as _, Result};
use clap::Subcommand;
use serde_json::{json, Value};

use crate::cli::Ctx;
use crate::client::sse;
use crate::output;

const DOCS_BASE: &str = "https://docs.wise.com";
const ASK_PATH: &str = "/_ask-ai";

#[derive(Debug, Subcommand)]
pub enum DocsCmd {
    /// Ask a question against the live Wise docs.
    Ask {
        /// The question to ask.
        question: String,
        /// Path to a JSON array of {role, content} history messages.
        #[arg(long)]
        history: Option<String>,
        /// Print the full response as a single JSON blob instead of streaming.
        #[arg(long)]
        no_stream: bool,
        /// Optional locale.
        #[arg(long, default_value = "default_locale")]
        locale: String,
    },
}

pub async fn run(cmd: DocsCmd, ctx: &Ctx) -> Result<()> {
    match cmd {
        DocsCmd::Ask {
            question,
            history,
            no_stream,
            locale,
        } => {
            let history_v: Value = if let Some(path) = history {
                let s = std::fs::read_to_string(&path)
                    .with_context(|| format!("reading history file {path}"))?;
                serde_json::from_str(&s).context("history must be a JSON array")?
            } else {
                json!([])
            };
            let body = json!({
                "text": question,
                "history": history_v,
                "locale": locale,
                "filter": [],
                "searchSessionId": uuid::Uuid::new_v4().to_string(),
            });

            let resp = ctx
                .client
                .post_sse(Some(DOCS_BASE), ASK_PATH, &body)
                .await
                .context("calling docs.wise.com /_ask-ai")?;

            if no_stream {
                // Collect everything into a single JSON object.
                let mut answer = String::new();
                let mut sources = Vec::new();
                let mut message_id: Option<String> = None;
                sse::stream_events(resp, |ev| {
                    match ev {
                        sse::AskAiEvent::Answer { answer: a } => answer.push_str(&a),
                        sse::AskAiEvent::Sources { sources: s } => sources = s,
                        sse::AskAiEvent::MessageId { message_id: id } => message_id = Some(id),
                        sse::AskAiEvent::Unknown => {}
                    }
                    Ok(())
                })
                .await?;
                output::print(
                    &json!({
                        "messageId": message_id,
                        "answer": answer,
                        "sources": sources,
                    }),
                    ctx.output(),
                );
            } else {
                // Stream answer chunks to stdout, then emit a JSON tail to stderr.
                let mut stdout = tokio::io::stdout();
                let sources = sse::stream_answer_to(resp, &mut stdout).await?;
                use tokio::io::AsyncWriteExt;
                stdout.write_all(b"\n").await.ok();
                stdout.flush().await.ok();
                if !sources.is_empty() {
                    let summary = json!({ "sources": sources });
                    eprintln!(
                        "{}",
                        serde_json::to_string(&summary).unwrap_or_default()
                    );
                }
            }
            Ok(())
        }
    }
}
