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
    pub truncate_threshold: usize,
    pub keep_recent: usize,
}

impl Default for CompressorConfig {
    fn default() -> Self {
        CompressorConfig {
            dedupe_system: true,
            collapse_whitespace: true,
            truncate_history: true,
            truncate_threshold: TRUNCATE_THRESHOLD,
            keep_recent: KEEP_RECENT,
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
            truncate_threshold: usize::MAX,
            keep_recent: usize::MAX,
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

    if config.collapse_whitespace {
        result.whitespace_chars_removed = collapse_whitespace(&mut request.messages);
    }

    if config.truncate_history {
        result.messages_truncated = truncate_history(
            &mut request.messages,
            config.truncate_threshold,
            config.keep_recent,
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
fn truncate_history(messages: &mut Vec<Message>, threshold: usize, keep_recent: usize) -> usize {
    if messages.len() <= threshold || keep_recent >= messages.len() {
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
}
