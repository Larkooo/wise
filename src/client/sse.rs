// Tiny Server-Sent Events parser for the docs ask-ai endpoint.
//
// We don't use a full SSE crate because the format we need to handle is
// minimal: each event is `event: <name>\ndata: <json>\n\n`. Wise's docs
// endpoint emits `event: data` with JSON payloads of three types:
// `messageId`, `sources`, and `answer` (with chunked text).

use anyhow::{anyhow, Result};
use futures_util::StreamExt;
use reqwest::Response;
use serde::Deserialize;
use serde_json::Value;
use tokio::io::{AsyncWrite, AsyncWriteExt};

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum AskAiEvent {
    MessageId {
        #[serde(rename = "messageId")]
        message_id: String,
    },
    Sources {
        sources: Vec<Source>,
    },
    Answer {
        answer: String,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Deserialize, serde::Serialize, Clone)]
pub struct Source {
    pub id: String,
    pub url: String,
    pub title: String,
}

/// Stream the SSE response, calling `on_event` for each parsed event.
pub async fn stream_events<F>(resp: Response, mut on_event: F) -> Result<()>
where
    F: FnMut(AskAiEvent) -> Result<()>,
{
    let mut stream = resp.bytes_stream();
    let mut buf: Vec<u8> = Vec::with_capacity(8 * 1024);

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        buf.extend_from_slice(&chunk);

        // Process any complete events (terminated by \n\n).
        loop {
            let Some(pos) = find_double_newline(&buf) else {
                break;
            };
            let raw_event = buf.drain(..pos + 2).collect::<Vec<u8>>();
            let event_str = std::str::from_utf8(&raw_event)
                .map_err(|e| anyhow!("non-utf8 SSE chunk: {e}"))?;
            if let Some(event) = parse_event(event_str)? {
                on_event(event)?;
            }
        }
    }
    Ok(())
}

/// Stream events and write the `answer` chunks to `out`. Returns the
/// collected sources at the end.
pub async fn stream_answer_to<W: AsyncWrite + Unpin>(
    resp: Response,
    out: &mut W,
) -> Result<Vec<Source>> {
    let mut sources = Vec::new();
    let mut stream = resp.bytes_stream();
    let mut buf: Vec<u8> = Vec::with_capacity(8 * 1024);

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        buf.extend_from_slice(&chunk);
        loop {
            let Some(pos) = find_double_newline(&buf) else {
                break;
            };
            let raw_event: Vec<u8> = buf.drain(..pos + 2).collect();
            let event_str = std::str::from_utf8(&raw_event)
                .map_err(|e| anyhow!("non-utf8 SSE chunk: {e}"))?;
            if let Some(event) = parse_event(event_str)? {
                match event {
                    AskAiEvent::Answer { answer } => {
                        out.write_all(answer.as_bytes()).await?;
                        out.flush().await?;
                    }
                    AskAiEvent::Sources { sources: srcs } => sources = srcs,
                    AskAiEvent::MessageId { .. } | AskAiEvent::Unknown => {}
                }
            }
        }
    }
    Ok(sources)
}

fn find_double_newline(buf: &[u8]) -> Option<usize> {
    buf.windows(2).position(|w| w == b"\n\n")
}

/// Parse a single `event: ...\ndata: ...\n` block. Returns None for
/// non-data events (e.g. heartbeats).
fn parse_event(raw: &str) -> Result<Option<AskAiEvent>> {
    let mut data_lines: Vec<&str> = Vec::new();
    for line in raw.split('\n') {
        let line = line.trim_end_matches('\r');
        if let Some(rest) = line.strip_prefix("data:") {
            data_lines.push(rest.trim_start());
        }
        // We ignore `event:`, `id:`, `retry:` — the type is inside the JSON.
    }
    if data_lines.is_empty() {
        return Ok(None);
    }
    let data = data_lines.join("\n");
    if data.is_empty() {
        return Ok(None);
    }
    let value: Value = serde_json::from_str(&data)
        .map_err(|e| anyhow!("invalid SSE data JSON: {e}\nraw: {data}"))?;
    let event: AskAiEvent = serde_json::from_value(value)
        .map_err(|e| anyhow!("unrecognized SSE event shape: {e}"))?;
    Ok(Some(event))
}
