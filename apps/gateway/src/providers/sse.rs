//! Server-sent event decoding for streaming responses.
//!
//! Every streaming provider speaks SSE, but HTTP chunk boundaries have nothing to do with
//! event boundaries: a single `data:` line routinely arrives split across two TCP reads,
//! and two events routinely arrive in one. [`SseDecoder`] buffers across chunks so an
//! event is only emitted once it is complete.
//!
//! Getting this wrong produces the worst class of streaming bug — occasional truncated
//! tokens under load, invisible in testing — so the decoder is tested against
//! byte-by-byte delivery.

/// Incremental SSE decoder.
#[derive(Debug, Default)]
pub struct SseDecoder {
    buffer: String,
}

impl SseDecoder {
    /// A fresh decoder.
    pub fn new() -> SseDecoder {
        SseDecoder::default()
    }

    /// Feed a chunk of bytes, returning every complete `data:` payload it completes.
    ///
    /// Comment lines (`:`), other SSE fields (`event:`, `id:`, `retry:`), and blank
    /// separators are consumed and produce nothing.
    pub fn push(&mut self, chunk: &str) -> Vec<String> {
        self.buffer.push_str(chunk);
        let mut payloads = Vec::new();

        // Only consume up to the last newline: anything after it is a partial line that
        // must wait for more bytes.
        while let Some(newline) = self.buffer.find('\n') {
            let line = self.buffer[..newline].trim_end_matches('\r').to_string();
            self.buffer.drain(..=newline);

            if let Some(payload) = line.strip_prefix("data:") {
                payloads.push(payload.trim().to_string());
            }
        }
        payloads
    }

    /// Flush any trailing partial line at end of stream.
    ///
    /// Well-behaved servers end with a newline, but a connection that closes mid-line
    /// would otherwise silently drop a final complete event.
    pub fn finish(&mut self) -> Vec<String> {
        if self.buffer.trim().is_empty() {
            self.buffer.clear();
            return Vec::new();
        }
        let remainder = std::mem::take(&mut self.buffer);
        remainder
            .lines()
            .filter_map(|line| line.strip_prefix("data:").map(|p| p.trim().to_string()))
            .collect()
    }

    /// True when nothing is buffered.
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }
}

/// The sentinel most OpenAI-compatible providers send to close a stream.
pub const DONE_SENTINEL: &str = "[DONE]";

/// True when a payload is the stream terminator rather than content.
pub fn is_done(payload: &str) -> bool {
    payload == DONE_SENTINEL
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_complete_events() {
        let mut decoder = SseDecoder::new();
        let events = decoder.push("data: {\"a\":1}\n\ndata: {\"b\":2}\n\n");
        assert_eq!(events, vec!["{\"a\":1}", "{\"b\":2}"]);
        assert!(decoder.is_empty());
    }

    #[test]
    fn buffers_events_split_across_chunks() {
        // The real-world failure: one JSON payload arriving in two TCP reads.
        let mut decoder = SseDecoder::new();
        assert!(decoder.push("data: {\"content\":\"hel").is_empty());
        let events = decoder.push("lo\"}\n\n");
        assert_eq!(events, vec!["{\"content\":\"hello\"}"]);
    }

    #[test]
    fn handles_byte_at_a_time_delivery() {
        // The pathological case. Every byte separately must still yield exactly the
        // original three events, uncorrupted.
        let input = "data: one\n\ndata: two\n\ndata: three\n\n";
        let mut decoder = SseDecoder::new();
        let mut collected = Vec::new();
        for ch in input.chars() {
            collected.extend(decoder.push(&ch.to_string()));
        }
        assert_eq!(collected, vec!["one", "two", "three"]);
    }

    #[test]
    fn handles_multiple_events_in_one_chunk() {
        let mut decoder = SseDecoder::new();
        let events = decoder.push("data: a\n\ndata: b\n\ndata: c\n\n");
        assert_eq!(events, vec!["a", "b", "c"]);
    }

    #[test]
    fn ignores_comments_and_other_sse_fields() {
        let mut decoder = SseDecoder::new();
        let events = decoder.push(": keep-alive\nevent: message\nid: 42\ndata: payload\n\n");
        assert_eq!(events, vec!["payload"]);
    }

    #[test]
    fn tolerates_crlf_line_endings() {
        let mut decoder = SseDecoder::new();
        let events = decoder.push("data: hello\r\n\r\n");
        assert_eq!(events, vec!["hello"]);
    }

    #[test]
    fn recognises_the_done_sentinel() {
        let mut decoder = SseDecoder::new();
        let events = decoder.push("data: [DONE]\n\n");
        assert_eq!(events, vec!["[DONE]"]);
        assert!(is_done(&events[0]));
        assert!(!is_done("{\"content\":\"x\"}"));
    }

    #[test]
    fn finish_flushes_a_trailing_line_without_a_newline() {
        // A provider that closes the connection without a final newline must not cost the
        // caller their last token.
        let mut decoder = SseDecoder::new();
        assert!(decoder.push("data: final chunk").is_empty());
        assert_eq!(decoder.finish(), vec!["final chunk"]);
        assert!(decoder.is_empty());
    }

    #[test]
    fn finish_on_a_clean_stream_yields_nothing() {
        let mut decoder = SseDecoder::new();
        decoder.push("data: x\n\n");
        assert!(decoder.finish().is_empty());
    }

    #[test]
    fn empty_data_lines_are_preserved_as_empty_payloads() {
        // Some providers send `data:` with no body as a keep-alive; the caller decides.
        let mut decoder = SseDecoder::new();
        assert_eq!(decoder.push("data:\n\n"), vec![""]);
    }

    #[test]
    fn no_content_is_lost_across_a_long_randomised_split() {
        // Property check: however the stream is chopped up, the decoded events are
        // identical to the ones decoded from the whole input at once.
        let payloads: Vec<String> = (0..200).map(|i| format!("{{\"i\":{i}}}")).collect();
        let input: String = payloads.iter().map(|p| format!("data: {p}\n\n")).collect();

        let mut whole = SseDecoder::new();
        let expected = whole.push(&input);
        assert_eq!(expected.len(), 200);

        for split_size in [1, 3, 7, 13, 64, 512] {
            let mut decoder = SseDecoder::new();
            let mut collected = Vec::new();
            let bytes: Vec<char> = input.chars().collect();
            for window in bytes.chunks(split_size) {
                collected.extend(decoder.push(&window.iter().collect::<String>()));
            }
            collected.extend(decoder.finish());
            assert_eq!(collected, expected, "content lost at split size {split_size}");
        }
    }
}
