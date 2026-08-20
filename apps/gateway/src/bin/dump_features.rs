//! Feature dumper for classifier training.
//!
//! Emits the exact feature vectors the gateway computes, alongside their labels, so the
//! V2 weights are fitted against **the same extraction code that runs in production**.
//!
//! Reimplementing feature extraction in the training script would be the obvious
//! shortcut and the obvious bug: the two copies drift, and the model is then fitted to
//! features nobody serves. Dumping from the real implementation makes that impossible.
//!
//! ```text
//! cargo run --bin dump_features > /tmp/features.json
//! python scripts/train_classifier.py /tmp/features.json
//! ```

use aegis_gateway::engine::classifier::Features;
use aegis_gateway::types::{Message, NormalizedRequest, Role};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
struct Case {
    name: String,
    #[serde(default)]
    system: String,
    prompt: String,
    #[serde(default)]
    history: Vec<String>,
    #[serde(default)]
    tools: bool,
    expected: String,
}

#[derive(Debug, Serialize)]
struct Row {
    name: String,
    label: String,
    features: Vec<f32>,
    /// Tool requests short-circuit before the linear model, so they are excluded from
    /// the fit rather than dragging the weights toward a decision never made this way.
    tool_shortcircuit: bool,
}

fn main() {
    let raw = include_str!("../../fixtures/classifier_cases.json");
    let cases: Vec<Case> = match serde_json::from_str(raw) {
        Ok(cases) => cases,
        Err(e) => {
            eprintln!("fixture parse failed: {e}");
            std::process::exit(1);
        }
    };

    let rows: Vec<Row> = cases
        .iter()
        .map(|case| {
            let mut messages = Vec::new();
            if !case.system.is_empty() {
                messages.push(Message::text(Role::System, case.system.clone()));
            }
            for (i, turn) in case.history.iter().enumerate() {
                let role = if i % 2 == 0 { Role::User } else { Role::Assistant };
                messages.push(Message::text(role, turn.clone()));
            }
            messages.push(Message::text(Role::User, case.prompt.clone()));

            let request = NormalizedRequest {
                messages,
                tools: if case.tools {
                    vec![serde_json::json!({"type": "function", "function": {"name": "f"}})]
                } else {
                    vec![]
                },
                ..NormalizedRequest::simple("gpt-4o", "")
            };

            Row {
                name: case.name.clone(),
                label: case.expected.clone(),
                features: Features::extract(&request).as_vector().to_vec(),
                tool_shortcircuit: case.tools,
            }
        })
        .collect();

    match serde_json::to_string_pretty(&rows) {
        Ok(json) => println!("{json}"),
        Err(e) => {
            eprintln!("serialization failed: {e}");
            std::process::exit(1);
        }
    }
}
