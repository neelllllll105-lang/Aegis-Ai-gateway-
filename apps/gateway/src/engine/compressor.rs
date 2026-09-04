//! Context compression — pipeline stage [6c].
//!
//! Long conversations carry a lot of tokens that buy nothing: the same system prompt
//! repeated by a framework on every turn, runs of whitespace from templated blocks, and
//! history so old the model will not use it. Removing that is the one saving that
//! requires no model substitution at all — the customer gets the same model, on a shorter
//! prompt, for less money.
//!
//! # The constraint that shapes every rule here
//!
//! Compression must be **lossless in effect**. We are editing someone's prompt, and a
//! model that answers differently because we quietly dropped a line is a far worse outcome
//! than a slightly larger bill. So each transformation below is conservative:
//!
//! * Only *exactly duplicated* system messages are removed, never merely similar ones.
//! * Whitespace is collapsed only where it cannot be significant — never inside a fenced
//!   code block, where indentation is the meaning.
//! * History is truncated only past a generous threshold, always keeps the system prompt
//!   and the most recent turns, and leaves an explicit marker so the model knows the
//!   history is partial rather than believing it is complete.

use crate::types::{Message, NormalizedRequest, Role};

/// Conversations longer than this are candidates for truncation.
pub const TRUNCATE_THRESHOLD: usize = 40;
/// Recent turns always preserved when truncating.
pub const KEEP_RECENT: usize = 20;
/// A block must be at least this many characters before duplicate-referencing or
/// stale-trimming will touch it. Below this the marker costs more than the saving.
pub const LARGE_BLOCK_CHARS: usize = 400;
/// Tool results older than this many messages from the end are candidates for trimming.
pub const STALE_TOOL_RESULT_AGE: usize = 6;
/// Head and tail retained on each side when a stale tool result is trimmed.
pub const TOOL_RESULT_KEEP_EDGE: usize = 300;
/// Marker inserted where history was removed.
pub const TRUNCATION_MARKER: &str =
    "[Earlier conversation history was omitted to fit the context window.]";

/// What compression achieved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CompressionResult {
    pub tokens_before: u64,
    pub tokens_after: u64,
    pub duplicate_system_messages_removed: usize,
    pub messages_truncated: usize,
    pub whitespace_chars_removed: usize,
    /// Pretty-printed JSON re-serialised compactly. Provably lossless: the value
    /// parses to exactly the same thing, only the formatting bytes are gone.
    pub json_blocks_minified: usize,
    /// Large blocks that exactly repeated an earlier one, replaced by a short pointer
    /// to it rather than deleted, so the turn structure the model reasons over survives.
    pub duplicate_blocks_referenced: usize,
    /// Stale tool results whose middle was elided, keeping head and tail.
    pub stale_tool_results_trimmed: usize,
}

impl CompressionResult {
    /// Tokens saved.
    pub fn tokens_saved(&self) -> u64 {
        self.tokens_before.saturating_sub(self.tokens_after)
    }

    /// True when anything changed.
    pub fn is_noop(&self) -> bool {
        self.tokens_saved() == 0
            && self.duplicate_system_messages_removed == 0
            && self.messages_truncated == 0
            && self.json_blocks_minified == 0
            && self.duplicate_blocks_referenced == 0
            && self.stale_tool_results_trimmed == 0
    }

    /// Proportion of tokens removed.
    pub fn savings_percent(&self) -> f64 {
        if self.tokens_before == 0 {
            return 0.0;
        }
        (self.tokens_saved() as f64 / self.tokens_before as f64) * 100.0
    }
}

/// Compression settings.
#[derive(Debug, Clone, Copy)]
pub struct CompressorConfig {
    pub dedupe_system: bool,
    pub collapse_whitespace: bool,
    pub truncate_history: bool,
    pub minify_json: bool,
    pub reference_duplicate_blocks: bool,
    pub trim_stale_tool_results: bool,
    pub truncate_threshold: usize,
    pub keep_recent: usize,
    /// Maximum target tokens before history truncation triggers, regardless of turn count.
    pub token_budget: Option<u64>,
}

impl Default for CompressorConfig {
    fn default() -> Self {
        CompressorConfig {
            dedupe_system: true,
            collapse_whitespace: true,
            truncate_history: true,
            minify_json: true,
            reference_duplicate_blocks: true,
            trim_stale_tool_results: true,
            truncate_threshold: TRUNCATE_THRESHOLD,
            keep_recent: KEEP_RECENT,
            token_budget: Some(32_000),
        }
    }
}

impl CompressorConfig {
    /// Every transformation disabled. Used for zero-retention organisations and for
    /// requests where fidelity matters more than cost.
    pub fn disabled() -> CompressorConfig {
        CompressorConfig {
            dedupe_system: false,
            collapse_whitespace: false,
            truncate_history: false,
            minify_json: false,
            reference_duplicate_blocks: false,
            trim_stale_tool_results: false,
            truncate_threshold: usize::MAX,
            keep_recent: usize::MAX,
            token_budget: None,
        }
    }
}

/// Compress a request in place, returning what was achieved.
pub fn compress(request: &mut NormalizedRequest, config: &CompressorConfig) -> CompressionResult {
    let mut result = CompressionResult {
        tokens_before: request.estimated_input_tokens(),
        ..Default::default()
    };

    if config.dedupe_system {
        result.duplicate_system_messages_removed = dedupe_system_messages(&mut request.messages);
    }

    // JSON first: minifying before the duplicate check means two payloads that differ
    // only in indentation are recognised as the identical value they are.
    if config.minify_json {
        result.json_blocks_minified = minify_json_blocks(&mut request.messages);
    }

    if config.reference_duplicate_blocks {
        result.duplicate_blocks_referenced = reference_duplicate_blocks(&mut request.messages);
    }

    if config.trim_stale_tool_results {
        result.stale_tool_results_trimmed = trim_stale_tool_results(
            &mut request.messages,
            STALE_TOOL_RESULT_AGE,
            TOOL_RESULT_KEEP_EDGE,
        );
    }

    if config.collapse_whitespace {
        result.whitespace_chars_removed = collapse_whitespace(&mut request.messages);
    }

    if config.truncate_history {
        result.messages_truncated = truncate_history(
            &mut request.messages,
            config.truncate_threshold,
            config.keep_recent,
            config.token_budget,
            result.tokens_before,
        );
    }

    result.tokens_after = request.estimated_input_tokens();
    result
}

/// Remove system messages whose text exactly duplicates an earlier one.
///
/// Agent frameworks routinely re-send the whole system prompt on every turn. On a long
/// conversation with a 2000-token system prompt, that is the single largest waste in the
/// request. Only exact duplicates go — a modified system message is a deliberate change.
fn dedupe_system_messages(messages: &mut Vec<Message>) -> usize {
    let mut seen: Vec<String> = Vec::new();
    let mut removed = 0;

    messages.retain(|message| {
        if !matches!(message.role, Role::System | Role::Developer) {
            return true;
        }
        let text = message.text_content();
        if text.is_empty() {
            return true;
        }
        if seen.contains(&text) {
            removed += 1;
            false
        } else {
            seen.push(text);
            true
        }
    });

    removed
}

/// Collapse runs of whitespace outside fenced code blocks.
fn collapse_whitespace(messages: &mut [Message]) -> usize {
    let mut removed = 0;
    for message in messages.iter_mut() {
        let Some(content) = &message.content else {
            continue;
        };
        let original = content.as_text();
        if original.is_empty() {
            continue;
        }
        let collapsed = collapse_outside_code_fences(&original);
        if collapsed.len() < original.len() {
            removed += original.len() - collapsed.len();
            message.content = Some(crate::types::Content::Text(collapsed));
        }
    }
    removed
}

/// Collapse whitespace runs, leaving fenced code blocks untouched.
///
/// Indentation inside a code fence is semantic — in Python it *is* the program — so the
/// fence state is tracked and content inside is copied verbatim.
fn collapse_outside_code_fences(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut in_fence = false;

    for (index, line) in input.split('\n').enumerate() {
        if index > 0 {
            out.push('\n');
        }
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            out.push_str(line);
            continue;
        }
        if in_fence {
            out.push_str(line);
            continue;
        }

        // Collapse internal runs of spaces and tabs, and drop trailing whitespace.
        let mut last_was_space = false;
        let mut collapsed = String::with_capacity(line.len());
        for ch in line.chars() {
            if ch == ' ' || ch == '\t' {
                if !last_was_space {
                    collapsed.push(' ');
                }
                last_was_space = true;
            } else {
                collapsed.push(ch);
                last_was_space = false;
            }
        }
        out.push_str(collapsed.trim_end());
    }

    // Collapse runs of three or more blank lines down to two.
    while out.contains("\n\n\n") {
        out = out.replace("\n\n\n", "\n\n");
    }
    out
}

/// Drop the middle of an over-long conversation, keeping system messages and recent turns.
///
/// Returns the number of messages removed.
fn truncate_history(
    messages: &mut Vec<Message>,
    threshold: usize,
    keep_recent: usize,
    token_budget: Option<u64>,
    estimated_tokens: u64,
) -> usize {
    let exceeds_turns = messages.len() > threshold;
    let exceeds_budget = token_budget.map(|b| estimated_tokens > b).unwrap_or(false);
    if (!exceeds_turns && !exceeds_budget) || keep_recent >= messages.len() {
        return 0;
    }

    // System messages are instructions, not history — they are never candidates.
    let system: Vec<Message> = messages
        .iter()
        .filter(|m| matches!(m.role, Role::System | Role::Developer))
        .cloned()
        .collect();
    let conversation: Vec<Message> = messages
        .iter()
        .filter(|m| !matches!(m.role, Role::System | Role::Developer))
        .cloned()
        .collect();

    if conversation.len() <= keep_recent {
        return 0;
    }

    let removed = conversation.len() - keep_recent;
    let recent = &conversation[conversation.len() - keep_recent..];

    let mut rebuilt = system;
    // Tell the model its history is partial. Without this it may assume the conversation
    // began at the first message it can see and answer with false confidence about what
    // was already discussed.
    rebuilt.push(Message::text(Role::System, TRUNCATION_MARKER));
    rebuilt.extend_from_slice(recent);

    *messages = rebuilt;
    removed
}

/// Whether a conversation is long enough to warrant an LLM-generated summary of the
/// dropped history (Phase 5, `P5.4`).
///
/// Summarization costs a model call, so it only pays off on genuinely long conversations,
/// and it never runs on the hot path — [`crate::workers`] produces the summary
/// asynchronously and the next request picks it up.
pub fn warrants_summarization(request: &NormalizedRequest) -> bool {
    request.messages.len() > 100 && request.estimated_input_tokens() > 20_000
}

/// Re-serialise pretty-printed JSON compactly.
///
/// The strongest guarantee in this module: a JSON value that round-trips through
/// `serde_json` is *the same value*. Only insignificant formatting bytes — indentation,
/// the space after `:` and `,`, trailing newlines — are removed. A model parsing the
/// result gets identical data.
///
/// This matters because tool results are overwhelmingly pretty-printed JSON, and
/// indentation is routinely 20-40% of such a payload. It is the largest safe saving
/// available on agentic traffic, and unlike every other transformation here it cannot
/// change meaning even in principle.
fn minify_json_blocks(messages: &mut [Message]) -> usize {
    let mut minified = 0;
    for message in messages.iter_mut() {
        let Some(content) = &message.content else {
            continue;
        };
        let original = content.as_text();
        let trimmed = original.trim();
        // Cheap gate before attempting a parse: only `{`/`[` can start a JSON document
        // worth minifying, and a short one has nothing to gain.
        if trimmed.len() < 64 || !(trimmed.starts_with('{') || trimmed.starts_with('[')) {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) else {
            continue;
        };
        let Ok(compact) = serde_json::to_string(&value) else {
            continue;
        };
        if compact.len() < trimmed.len() {
            message.content = Some(crate::types::Content::Text(compact));
            minified += 1;
        }
    }
    minified
}

/// Replace a large block that exactly repeats an earlier one with a pointer to it.
///
/// A retrieval pipeline re-injecting the same document, or an agent re-reading a file it
/// already read, sends the identical payload several times in one conversation. Deleting
/// the later copy would change the turn structure the model reasons over; replacing its
/// body with a one-line reference keeps the structure and drops the tokens.
///
/// Only *byte-identical* bodies are referenced — never merely similar ones — and only
/// above [`LARGE_BLOCK_CHARS`], below which the marker would cost more than it saves.
/// System messages are excluded because [`dedupe_system_messages`] already handles them.
fn reference_duplicate_blocks(messages: &mut [Message]) -> usize {
    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut referenced = 0;

    for (index, message) in messages.iter_mut().enumerate() {
        if matches!(message.role, Role::System | Role::Developer) {
            continue;
        }
        let Some(content) = &message.content else {
            continue;
        };
        let text = content.as_text();
        if text.len() < LARGE_BLOCK_CHARS {
            continue;
        }
        match seen.get(&text) {
            Some(first) => {
                let marker = format!(
                    "[Identical to the content already provided in message {} of this \
                     conversation; omitted here to save context.]",
                    first + 1
                );
                message.content = Some(crate::types::Content::Text(marker));
                referenced += 1;
            }
            None => {
                seen.insert(text, index);
            }
        }
    }

    referenced
}

/// Elide the middle of large, stale tool results.
///
/// In an agentic loop the context fills with tool output that has already served its
/// purpose — a file read fifteen steps ago, a search result long since acted on. The
/// recent ones are load-bearing and are never touched; older large ones keep their head
/// and tail (where identifying detail and conclusions live) and lose the middle, with a
/// marker stating exactly how much was removed so the model knows the payload is partial.
///
/// Deliberately not applied to the most recent [`STALE_TOOL_RESULT_AGE`] messages: the
/// tool output a model is actively reasoning about must arrive whole.
fn trim_stale_tool_results(messages: &mut [Message], stale_age: usize, keep_edge: usize) -> usize {
    let total = messages.len();
    if total <= stale_age {
        return 0;
    }
    let cutoff = total - stale_age;
    let mut trimmed = 0;

    for message in messages.iter_mut().take(cutoff) {
        if !matches!(message.role, Role::Tool) {
            continue;
        }
        let Some(content) = &message.content else {
            continue;
        };
        let text = content.as_text();
        // Needs to be long enough that removing the middle beats the marker's own cost.
        if text.chars().count() < keep_edge * 2 + LARGE_BLOCK_CHARS {
            continue;
        }

        let chars: Vec<char> = text.chars().collect();
        let head: String = chars[..keep_edge].iter().collect();
        let tail: String = chars[chars.len() - keep_edge..].iter().collect();
        let removed = chars.len() - (keep_edge * 2);
        let rebuilt = format!(
            "{head}\n[... {removed} characters of earlier tool output omitted to save \
             context ...]\n{tail}"
        );
        message.content = Some(crate::types::Content::Text(rebuilt));
        trimmed += 1;
    }

    trimmed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(role: Role, text: &str) -> Message {
        Message::text(role, text)
    }

    #[test]
    fn duplicate_system_prompts_are_removed() {
        // The single biggest real-world waste: a framework re-sending the system prompt.
        let mut request = NormalizedRequest {
            messages: vec![
                message(Role::System, "You are a helpful assistant with many rules."),
                message(Role::User, "hi"),
                message(Role::System, "You are a helpful assistant with many rules."),
                message(Role::Assistant, "hello"),
                message(Role::System, "You are a helpful assistant with many rules."),
                message(Role::User, "bye"),
            ],
            ..NormalizedRequest::simple("gpt-4o", "")
        };

        let result = compress(&mut request, &CompressorConfig::default());
        assert_eq!(result.duplicate_system_messages_removed, 2);
        assert_eq!(
            request
                .messages
                .iter()
                .filter(|m| m.role == Role::System)
                .count(),
            1
        );
        assert!(result.tokens_saved() > 0);
    }

    #[test]
    fn different_system_prompts_are_all_kept() {
        // Only exact duplicates go. A changed system message is a deliberate instruction.
        let mut request = NormalizedRequest {
            messages: vec![
                message(Role::System, "You are helpful."),
                message(Role::System, "You are helpful and terse."),
                message(Role::User, "hi"),
            ],
            ..NormalizedRequest::simple("gpt-4o", "")
        };
        let result = compress(&mut request, &CompressorConfig::default());
        assert_eq!(result.duplicate_system_messages_removed, 0);
        assert_eq!(request.messages.len(), 3);
    }

    #[test]
    fn code_block_indentation_is_never_touched() {
        // Collapsing whitespace inside a fence would silently corrupt Python.
        let python = "Here is code:\n```python\ndef f():\n    if x:\n        return 1\n```\nDone";
        let mut request = NormalizedRequest {
            messages: vec![message(Role::User, python)],
            ..NormalizedRequest::simple("gpt-4o", "")
        };
        compress(&mut request, &CompressorConfig::default());

        let text = request.messages[0].text_content();
        assert!(text.contains("    if x:"), "indentation lost:\n{text}");
        assert!(
            text.contains("        return 1"),
            "indentation lost:\n{text}"
        );
    }

    #[test]
    fn whitespace_outside_code_is_collapsed() {
        let mut request = NormalizedRequest {
            messages: vec![message(
                Role::User,
                "This    has     lots   of     spaces   and trailing ones.   ",
            )],
            ..NormalizedRequest::simple("gpt-4o", "")
        };
        let result = compress(&mut request, &CompressorConfig::default());
        let text = request.messages[0].text_content();
        assert!(!text.contains("  "), "runs remain: {text:?}");
        assert!(
            text.ends_with("ones."),
            "trailing whitespace remains: {text:?}"
        );
        assert!(result.whitespace_chars_removed > 0);
    }

    #[test]
    fn excessive_blank_lines_are_reduced() {
        let mut request = NormalizedRequest {
            messages: vec![message(Role::User, "a\n\n\n\n\n\nb")],
            ..NormalizedRequest::simple("gpt-4o", "")
        };
        compress(&mut request, &CompressorConfig::default());
        assert_eq!(request.messages[0].text_content(), "a\n\nb");
    }

    #[test]
    fn short_conversations_are_not_truncated() {
        let mut request = NormalizedRequest {
            messages: (0..10)
                .map(|i| message(Role::User, &format!("turn {i}")))
                .collect(),
            ..NormalizedRequest::simple("gpt-4o", "")
        };
        let result = compress(&mut request, &CompressorConfig::default());
        assert_eq!(result.messages_truncated, 0);
        assert_eq!(request.messages.len(), 10);
    }

    #[test]
    fn long_conversations_keep_the_system_prompt_and_recent_turns() {
        let mut messages = vec![message(Role::System, "You are helpful.")];
        for i in 0..60 {
            messages.push(message(Role::User, &format!("question {i}")));
        }
        let mut request = NormalizedRequest {
            messages,
            ..NormalizedRequest::simple("gpt-4o", "")
        };

        let result = compress(&mut request, &CompressorConfig::default());
        assert_eq!(result.messages_truncated, 40);

        // The system prompt survives.
        assert_eq!(request.messages[0].text_content(), "You are helpful.");
        // The model is told the history is partial.
        assert!(request
            .messages
            .iter()
            .any(|m| m.text_content() == TRUNCATION_MARKER));
        // The most recent turn survives.
        assert_eq!(
            request.messages.last().unwrap().text_content(),
            "question 59"
        );
        // And the oldest is gone.
        assert!(!request
            .messages
            .iter()
            .any(|m| m.text_content() == "question 0"));
    }

    #[test]
    fn truncation_marker_prevents_false_confidence() {
        // Without the marker the model believes it can see the whole conversation.
        let mut request = NormalizedRequest {
            messages: (0..60)
                .map(|i| message(Role::User, &format!("t{i}")))
                .collect(),
            ..NormalizedRequest::simple("gpt-4o", "")
        };
        compress(&mut request, &CompressorConfig::default());
        assert!(request
            .messages
            .iter()
            .any(|m| m.text_content() == TRUNCATION_MARKER));
    }

    #[test]
    fn disabled_config_changes_nothing() {
        // Zero-retention organisations get their prompt delivered exactly as written.
        let original = NormalizedRequest {
            messages: vec![
                message(Role::System, "dup"),
                message(Role::System, "dup"),
                message(Role::User, "lots     of    space"),
            ],
            ..NormalizedRequest::simple("gpt-4o", "")
        };
        let mut request = original.clone();
        let result = compress(&mut request, &CompressorConfig::disabled());
        assert_eq!(request, original);
        assert!(result.is_noop());
    }

    #[test]
    fn compression_reports_accurate_token_deltas() {
        let mut request = NormalizedRequest {
            messages: vec![
                message(Role::System, &"a very long system prompt ".repeat(50)),
                message(Role::User, "hi"),
                message(Role::System, &"a very long system prompt ".repeat(50)),
            ],
            ..NormalizedRequest::simple("gpt-4o", "")
        };
        let before = request.estimated_input_tokens();
        let result = compress(&mut request, &CompressorConfig::default());

        assert_eq!(result.tokens_before, before);
        assert_eq!(result.tokens_after, request.estimated_input_tokens());
        assert!(result.tokens_after < result.tokens_before);
        assert!(
            result.savings_percent() > 30.0,
            "{}%",
            result.savings_percent()
        );
    }

    #[test]
    fn compression_never_increases_token_count() {
        // The property that matters: compression must never make a request more expensive.
        let cases = vec![
            vec![message(Role::User, "short")],
            vec![message(Role::User, "```\ncode\n```")],
            vec![message(Role::System, "s"), message(Role::User, "u")],
            (0..80)
                .map(|i| message(Role::User, &format!("m{i}")))
                .collect(),
            vec![message(Role::User, "")],
        ];
        for messages in cases {
            let mut request = NormalizedRequest {
                messages,
                ..NormalizedRequest::simple("gpt-4o", "")
            };
            let result = compress(&mut request, &CompressorConfig::default());
            assert!(
                result.tokens_after <= result.tokens_before,
                "compression grew the request: {} -> {}",
                result.tokens_before,
                result.tokens_after
            );
        }
    }

    #[test]
    fn empty_and_degenerate_inputs_do_not_panic() {
        let mut empty = NormalizedRequest {
            messages: vec![],
            ..NormalizedRequest::simple("gpt-4o", "")
        };
        let result = compress(&mut empty, &CompressorConfig::default());
        assert!(result.is_noop());
        assert_eq!(result.savings_percent(), 0.0);
    }

    #[test]
    fn unterminated_code_fence_is_handled_safely() {
        // A malformed fence must not cause the rest of the message to be mangled or lost.
        let mut request = NormalizedRequest {
            messages: vec![message(Role::User, "text\n```\ncode    here\nmore   code")],
            ..NormalizedRequest::simple("gpt-4o", "")
        };
        compress(&mut request, &CompressorConfig::default());
        let text = request.messages[0].text_content();
        assert!(
            text.contains("code    here"),
            "content inside the fence was altered: {text:?}"
        );
        assert!(text.contains("more   code"));
    }

    #[test]
    fn summarization_is_reserved_for_very_long_conversations() {
        let short = NormalizedRequest::simple("gpt-4o", "hi");
        assert!(!warrants_summarization(&short));

        // Long in turns but not in tokens: summarization would cost a model call to save
        // very little, so it must not fire.
        let chatty = NormalizedRequest {
            messages: (0..150)
                .map(|i| message(Role::User, &format!("turn {i}")))
                .collect(),
            ..NormalizedRequest::simple("gpt-4o", "")
        };
        assert!(!warrants_summarization(&chatty));

        // Both thresholds crossed: >100 turns and >20k estimated tokens.
        let long = NormalizedRequest {
            messages: (0..150)
                .map(|i| message(Role::User, &format!("{} {}", "word ".repeat(120), i)))
                .collect(),
            ..NormalizedRequest::simple("gpt-4o", "")
        };
        assert!(
            long.estimated_input_tokens() > 20_000,
            "{}",
            long.estimated_input_tokens()
        );
        assert!(warrants_summarization(&long));
    }

    // ---------------------------------------------------------------------------
    // JSON minification — the provably-lossless transformation.
    // ---------------------------------------------------------------------------

    #[test]
    fn pretty_printed_json_is_minified_to_the_same_value() {
        let pretty = serde_json::to_string_pretty(&serde_json::json!({
            "results": [
                {"file": "src/main.rs", "line": 42, "match": "fn main"},
                {"file": "src/lib.rs", "line": 7, "match": "pub mod"}
            ],
            "truncated": false
        }))
        .unwrap();

        let mut request = NormalizedRequest {
            messages: vec![message(Role::Tool, &pretty)],
            ..NormalizedRequest::simple("gpt-4o", "")
        };
        let result = compress(&mut request, &CompressorConfig::default());

        assert_eq!(result.json_blocks_minified, 1);
        let after = request.messages[0].text_content();
        assert!(after.len() < pretty.len(), "minified form must be shorter");

        let before_value: serde_json::Value = serde_json::from_str(&pretty).unwrap();
        let after_value: serde_json::Value = serde_json::from_str(&after).unwrap();
        assert_eq!(
            before_value, after_value,
            "minification must never change the parsed value"
        );
    }

    #[test]
    fn text_that_merely_starts_with_a_brace_is_left_alone() {
        let prose = "{ this is not JSON, it is a sentence that happens to open with a brace \
                     and continues for a while so it clears the length gate comfortably. }";
        let mut request = NormalizedRequest {
            messages: vec![message(Role::User, prose)],
            ..NormalizedRequest::simple("gpt-4o", "")
        };
        let result = compress(&mut request, &CompressorConfig::default());
        assert_eq!(result.json_blocks_minified, 0);
        assert_eq!(request.messages[0].text_content(), prose);
    }

    // ---------------------------------------------------------------------------
    // Duplicate large blocks.
    // ---------------------------------------------------------------------------

    #[test]
    fn a_repeated_large_block_is_replaced_by_a_reference_not_deleted() {
        let document = "SECTION ".repeat(80);
        let mut request = NormalizedRequest {
            messages: vec![
                message(Role::User, &document),
                message(Role::Assistant, "Understood."),
                message(Role::User, &document),
            ],
            ..NormalizedRequest::simple("gpt-4o", "")
        };
        let result = compress(&mut request, &CompressorConfig::default());

        assert_eq!(result.duplicate_blocks_referenced, 1);
        assert_eq!(request.messages.len(), 3);
        assert!(request.messages[2].text_content().contains("Identical to"));
        assert!(result.tokens_saved() > 0);
    }

    #[test]
    fn a_small_repeated_block_is_left_alone() {
        let mut request = NormalizedRequest {
            messages: vec![
                message(Role::User, "hello there"),
                message(Role::Assistant, "hi"),
                message(Role::User, "hello there"),
            ],
            ..NormalizedRequest::simple("gpt-4o", "")
        };
        let result = compress(&mut request, &CompressorConfig::default());
        assert_eq!(result.duplicate_blocks_referenced, 0);
        assert_eq!(request.messages[2].text_content(), "hello there");
    }

    // ---------------------------------------------------------------------------
    // Stale tool results.
    // ---------------------------------------------------------------------------

    #[test]
    fn a_stale_tool_result_keeps_its_head_and_tail() {
        let payload = format!("HEAD-MARKER{}TAIL-MARKER", "x".repeat(4000));
        let mut messages = vec![message(Role::Tool, &payload)];
        for i in 0..10 {
            messages.push(message(Role::User, &format!("follow up {i}")));
        }
        let mut request = NormalizedRequest {
            messages,
            ..NormalizedRequest::simple("gpt-4o", "")
        };
        let result = compress(&mut request, &CompressorConfig::default());

        assert_eq!(result.stale_tool_results_trimmed, 1);
        let trimmed = request.messages[0].text_content();
        assert!(trimmed.contains("HEAD-MARKER"), "head must survive");
        assert!(trimmed.contains("TAIL-MARKER"), "tail must survive");
        assert!(trimmed.contains("omitted to save context"));
        assert!(trimmed.len() < payload.len());
    }

    #[test]
    fn recent_tool_results_are_never_trimmed() {
        let payload = "y".repeat(5000);
        let mut request = NormalizedRequest {
            messages: vec![
                message(Role::User, "run the search"),
                message(Role::Tool, &payload),
            ],
            ..NormalizedRequest::simple("gpt-4o", "")
        };
        let result = compress(&mut request, &CompressorConfig::default());
        assert_eq!(result.stale_tool_results_trimmed, 0);
        assert_eq!(request.messages[1].text_content(), payload);
    }

    #[test]
    fn every_new_technique_is_off_when_compression_is_disabled() {
        let pretty =
            serde_json::to_string_pretty(&serde_json::json!({"a": [1, 2, 3, 4, 5]})).unwrap();
        let document = "SECTION ".repeat(80);
        let mut request = NormalizedRequest {
            messages: vec![
                message(Role::Tool, &pretty),
                message(Role::User, &document),
                message(Role::User, &document),
            ],
            ..NormalizedRequest::simple("gpt-4o", "")
        };
        let before = request.messages.clone();
        let result = compress(&mut request, &CompressorConfig::disabled());

        assert!(result.is_noop());
        assert_eq!(result.json_blocks_minified, 0);
        assert_eq!(result.duplicate_blocks_referenced, 0);
        assert_eq!(result.stale_tool_results_trimmed, 0);
        assert_eq!(request.messages, before, "prompt must be byte-identical");
    }

    #[test]
    fn code_fences_survive_every_technique_together() {
        let code = "Here is the fix:\n```python\ndef f():\n    if x:\n        return 1\n```\n";
        let mut request = NormalizedRequest {
            messages: vec![message(Role::Assistant, code)],
            ..NormalizedRequest::simple("gpt-4o", "")
        };
        compress(&mut request, &CompressorConfig::default());
        let after = request.messages[0].text_content();
        assert!(
            after.contains("    if x:"),
            "4-space indent must survive: {after}"
        );
        assert!(
            after.contains("        return 1"),
            "8-space indent must survive"
        );
    }

    #[test]
    fn truncation_triggers_on_token_budget_exceeded() {
        // A conversation with only 8 messages (< TRUNCATE_THRESHOLD = 40)
        // but 50,000 tokens (> token_budget = 10,000)
        let large_msg = "word ".repeat(3000); // ~3750 tokens each
        let mut request = NormalizedRequest {
            messages: vec![
                message(Role::System, "You are an assistant."),
                message(Role::User, &large_msg),
                message(Role::Assistant, &large_msg),
                message(Role::User, &large_msg),
                message(Role::Assistant, &large_msg),
                message(Role::User, &large_msg),
                message(Role::Assistant, &large_msg),
                message(Role::User, "final question"),
            ],
            ..NormalizedRequest::simple("gpt-4o", "")
        };

        let config = CompressorConfig {
            token_budget: Some(10_000),
            truncate_threshold: 40,
            keep_recent: 3,
            ..CompressorConfig::default()
        };

        let result = compress(&mut request, &config);
        assert!(result.messages_truncated > 0);
        assert!(result.tokens_saved() > 0);
        assert!(request
            .messages
            .iter()
            .any(|m| m.text_content() == TRUNCATION_MARKER));
    }
}
