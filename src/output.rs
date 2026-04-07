// Output formatting. JSON by default (one line, agent-friendly), --pretty for
// indented JSON, --table for human-readable tables (only meaningful for some
// commands; falls back to pretty JSON otherwise).

use crate::cli::GlobalArgs;
use crate::client::WiseError;
use serde::Serialize;
use serde_json::Value;
use std::io::Write;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Json,
    Pretty,
    Table,
}

/// Print a serializable value to stdout in the requested format.
pub fn print<T: Serialize>(value: &T, format: OutputFormat) {
    let v = match serde_json::to_value(value) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{{\"error\":{{\"code\":\"serialize_failed\",\"message\":\"{e}\"}}}}");
            return;
        }
    };
    print_value(&v, format);
}

pub fn print_value(value: &Value, format: OutputFormat) {
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    match format {
        OutputFormat::Json => {
            let _ = writeln!(out, "{}", value);
        }
        OutputFormat::Pretty => {
            let s = serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string());
            let _ = writeln!(out, "{}", s);
        }
        OutputFormat::Table => match value_to_table(value) {
            Some(table) => {
                let _ = writeln!(out, "{}", table);
            }
            None => {
                let s = serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string());
                let _ = writeln!(out, "{}", s);
            }
        },
    }
}

/// Print an error to stderr in the requested format and as JSON-shaped errors.
pub fn print_error(err: &anyhow::Error, args: &GlobalArgs) {
    let format = args.output_format();
    if let Some(wise) = err.downcast_ref::<WiseError>() {
        let json = wise.to_json();
        match format {
            OutputFormat::Pretty | OutputFormat::Table => {
                let s = serde_json::to_string_pretty(&json).unwrap_or_else(|_| json.to_string());
                eprintln!("{}", s);
            }
            OutputFormat::Json => {
                eprintln!("{}", json);
            }
        }
        return;
    }
    let json = serde_json::json!({
        "error": {
            "code": "cli_error",
            "message": err.to_string(),
            "chain": err.chain().skip(1).map(|c| c.to_string()).collect::<Vec<_>>(),
        }
    });
    match format {
        OutputFormat::Pretty | OutputFormat::Table => {
            let s = serde_json::to_string_pretty(&json).unwrap_or_else(|_| json.to_string());
            eprintln!("{}", s);
        }
        OutputFormat::Json => {
            eprintln!("{}", json);
        }
    }
}

/// Best-effort table renderer for arrays of flat objects.
fn value_to_table(value: &Value) -> Option<String> {
    use comfy_table::{presets::UTF8_FULL, Cell, Table};
    let array = match value {
        Value::Array(arr) => arr,
        Value::Object(map) => {
            // Some endpoints wrap a list in an object — try common keys.
            for key in &["items", "data", "results", "balances", "transfers", "subscriptions"] {
                if let Some(Value::Array(arr)) = map.get(*key) {
                    return value_to_table(&Value::Array(arr.clone()));
                }
            }
            return None;
        }
        _ => return None,
    };
    if array.is_empty() {
        return Some("(empty)".to_string());
    }
    // Build columns from the union of top-level keys of the first few rows.
    let mut headers: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for row in array.iter().take(5) {
        if let Value::Object(map) = row {
            for k in map.keys() {
                if !seen.contains(k) {
                    seen.insert(k.clone());
                    headers.push(k.clone());
                }
            }
        }
    }
    if headers.is_empty() {
        return None;
    }
    let mut table = Table::new();
    table.load_preset(UTF8_FULL);
    table.set_header(headers.iter().map(|h| Cell::new(h)));
    for row in array {
        if let Value::Object(map) = row {
            let cells: Vec<Cell> = headers
                .iter()
                .map(|h| {
                    let v = map.get(h).cloned().unwrap_or(Value::Null);
                    Cell::new(stringify_cell(&v))
                })
                .collect();
            table.add_row(cells);
        }
    }
    Some(table.to_string())
}

fn stringify_cell(v: &Value) -> String {
    match v {
        Value::Null => "—".to_string(),
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        other => other.to_string(),
    }
}
