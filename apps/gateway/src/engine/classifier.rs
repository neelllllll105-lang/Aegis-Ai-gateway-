//! Request complexity classification — pipeline stage [6a].
//!
//! The classifier answers one question: **can a cheaper model handle this without the
//! user noticing?** Get it wrong in one direction and we burn money on a premium model
//! for "what is 2+2"; get it wrong in the other and we serve a nano model a
//! multi-file refactor, which is the failure that loses the customer.
//!
//! The asymmetry matters, and it shapes every weight below: *under*-estimating complexity
//! is far more damaging than over-estimating it. Part 5 says savings come from the
//! simple-request long tail, not from squeezing hard requests onto weak models.
//!
//! # Versions
//!
//! * [`ClassifierVersion::V1`] — hand-tuned heuristics. Deterministic, allocation-light,
//!   and explainable, which matters when a customer asks why their request was
//!   downgraded.
//! * [`ClassifierVersion::V2`] — the same extracted features fed through a linear model
//!   with committed weights (Phase 5). Trained offline; scoring stays in-process, with no
//!   extra network call in the hot path.
//!
//! Both consume the identical [`Features`] vector, so a version change is a scoring
//! change only — feature extraction, and therefore the explanation shown to a customer,
//! stays the same.

use crate::types::{Complexity, NormalizedRequest};

/// Words that signal genuine reasoning work rather than recall or formatting.
const REASONING_TERMS: &[&str] = &[
    "analyze",
    "analyse",
    "reason",
    "prove",
    "derive",
    "debug",
    "architect",
    "design",
    "optimize",
    "optimise",
    "refactor",
    "diagnose",
    "investigate",
    "evaluate",
    "compare",
    "trade-off",
    "tradeoff",
    "root cause",
    "why does",
    "why is",
    "explain how",
    "step by step",
    "think through",
    "strategy",
    "algorithm",
    "complexity analysis",
];

/// Words that signal a mechanical, low-difficulty task.
const TRIVIAL_TERMS: &[&str] = &[
    "translate",
    "spell",
    "capitalize",
    "uppercase",
    "lowercase",
    "rephrase",
    "reword",
    "what is the capital",
    "convert",
    "format as",
    "list the",
    "define ",
    "synonym",
    "antonym",
    "abbreviation",
    "emoji",
    "tl;dr",
    "in one word",
    "yes or no",
];

/// Phrases asking for explanation, comparison, or drafting. Not trivial recall, not deep
/// reasoning — the middle band, and the largest source of routable savings.
const EXPLANATORY_TERMS: &[&str] = &[
    "explain",
    "describe",
    "difference between",
    "differences between",
    "pros and cons",
    "compare",
    "summarize",
    "summarise",
    "outline",
    "draft ",
    "write an email",
    "how does",
    "how do i",
    "what are the",
    "walk me through",
    "overview of",
    "best practice",
    "when should i",
    "give an example",
];

/// Connectives that chain one request into several. "Analyze X, diagnose Y, and
/// architect Z" is three tasks in one turn, and a model that handles each in isolation
/// can still fail to hold them together.
const CHAINING_TERMS: &[&str] = &[
    ", and ",
    ", then ",
    " and then ",
    " after that",
    " finally,",
    " also ",
    "1.",
    "2.",
    " first, ",
    " second, ",
    " next, ",
    " and ",
];

/// Provider preferences by task domain, used by the router to break price ties
/// toward a provider known to excel at this class of work.
///
/// Soft guidance only: the router still applies health and capability filters first.
/// The domain preference just influences tie-breaking among equally-capable candidates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskDomain {
    /// Code generation, refactoring, debugging, test writing.
    Code,
    /// Multi-step reasoning, math, analysis, root-cause investigation.
    Reasoning,
    /// Natural language tasks: translation, summarisation, creative writing, drafting.
    Language,
    /// General / conversational — no strong signal either way.
    General,
}

impl TaskDomain {
    /// The provider(s) to prefer for this domain, in preference order.
    ///
    /// This is empirical guidance, not a hard rule — the router will still use any
    /// capable provider when the preferred one is down or too expensive.
    pub fn preferred_providers(&self) -> &'static [&'static str] {
        match self {
            // DeepSeek and Groq are fast, cheap, and strong on code.
            TaskDomain::Code => &["deepseek", "groq", "mistral"],
            // Google and Anthropic have long context windows and strong reasoning.
            TaskDomain::Reasoning => &["google", "vertex", "anthropic"],
            // Mistral leads on multilingual quality; Groq is a fast fallback.
            TaskDomain::Language => &["mistral", "groq", "google"],
            // No strong preference — let price and health decide.
            TaskDomain::General => &[],
        }
    }

    /// Short label for routing explanations and logs.
    pub fn as_str(&self) -> &'static str {
        match self {
            TaskDomain::Code => "code",
            TaskDomain::Reasoning => "reasoning",
            TaskDomain::Language => "language",
            TaskDomain::General => "general",
        }
    }
}

/// Verbs that, combined with code vocabulary, mean "produce or change code" — a
/// materially harder task than answering a question about code.
const CODE_INTENT_VERBS: &[&str] = &[
    "write a",
    "write the",
    "implement",
    "create a",
    "build a",
    "generate a",
    "add a",
    "fix the",
    "fix this",
    "refactor",
    "rewrite",
    "port ",
    "migrate",
    "extend the",
];

/// Tokens that indicate code is present or expected.
const CODE_TERMS: &[&str] = &[
    "function",
    "class ",
    "import ",
    "def ",
    "const ",
    "return",
    "async ",
    "await",
    "select ",
    "from ",
    "where ",
    "null",
    "undefined",
    "exception",
    "stack trace",
    "traceback",
    "compile",
    "segfault",
    "npm ",
    "cargo ",
    "pip ",
    "docker",
    "kubernetes",
    "regex",
    "api endpoint",
    "unit test",
    "sql",
    "query",
    "python",
    "javascript",
    "typescript",
    "rust",
    "css",
    "html",
    "endpoint",
    "database",
    "schema",
    "test case",
    "component",
    "middleware",
    "deployment",
];

/// Extracted, normalised signals. Every field is in `0.0..=1.0` so weights are directly
/// comparable and a trained model can consume the same vector.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Features {
    /// Input size, saturating at ~3000 tokens.
    pub length: f32,
    /// Conversation depth, saturating at 16 turns.
    pub turns: f32,
    /// Fenced code blocks present.
    pub code_block: f32,
    /// Code-adjacent vocabulary density.
    pub code_terms: f32,
    /// A request to produce or modify code, rather than merely discuss it.
    pub code_intent: f32,
    /// Several distinct tasks chained into one request.
    pub multi_step: f32,
    /// Explanation, comparison, or drafting requested.
    pub explanatory: f32,
    /// Density of reasoning verbs, saturating at ~4 distinct terms.
    pub reasoning: f32,
    /// System prompt length, saturating at 800 characters.
    pub system_length: f32,
    /// Mechanical-task vocabulary present.
    pub trivial: f32,
    /// The final user message is a single short question.
    pub short_question: f32,
    /// Tool calling requested.
    pub tools: f32,
}

impl Features {
    /// Extract features from a request.
    ///
    /// Single pass over lowercased text: the hot path budget for the whole routing stage
    /// is 0.5ms, and this runs on every uncached request.
    pub fn extract(request: &NormalizedRequest) -> Features {
        let text = request.all_text();
        let lowered = text.to_lowercase();
        let system = request.system_text();
        let last_user = request.last_user_message().unwrap_or_default();

        let tokens = request.estimated_input_tokens() as f32;
        let turns = request.messages.len() as f32;

        let code_block = if text.contains("```") { 1.0 } else { 0.0 };
        let code_hits = CODE_TERMS.iter().filter(|t| lowered.contains(**t)).count() as f32;
        let reasoning_hits = REASONING_TERMS
            .iter()
            .filter(|t| lowered.contains(**t))
            .count() as f32;
        let has_code_intent = (code_hits > 0.0 || text.contains("```"))
            && CODE_INTENT_VERBS.iter().any(|v| lowered.contains(*v));
        let trivial_hits = TRIVIAL_TERMS
            .iter()
            .filter(|t| lowered.contains(**t))
            .count() as f32;
        let is_explanatory = EXPLANATORY_TERMS.iter().any(|t| lowered.contains(*t));

        // Multi-step is the conjunction of two independent signals: several demanding
        // verbs AND the connectives that chain them. Either alone is common in ordinary
        // prose; together they reliably mark a compound task.
        let chaining_hits = CHAINING_TERMS
            .iter()
            .filter(|t| lowered.contains(**t))
            .count() as f32;
        let multi_step_signal = if reasoning_hits >= 3.0 && chaining_hits >= 1.0 {
            1.0
        } else if reasoning_hits >= 2.0 && chaining_hits >= 1.0 {
            0.6
        } else {
            0.0
        };

        // A short final question with no code and no multi-part structure is the
        // signature of the long tail we most want to route cheaply.
        let short_question = {
            let words = last_user.split_whitespace().count();
            let no_code = !last_user.contains("```");
            // Either an actual short question, or a very short instruction. An imperative
            // of ordinary length ("write a function that...") is neither, and treating it
            // as one was under-scoring code generation.
            let asks_a_question = last_user.trim_end().ends_with('?') && words <= 14;
            let very_short = words <= 6;
            let substantive = is_explanatory || has_code_intent;
            if no_code && !substantive && (asks_a_question || very_short) {
                1.0
            } else {
                0.0
            }
        };

        Features {
            length: clamp01(tokens / 3_000.0),
            turns: clamp01(turns / 16.0),
            code_block,
            code_terms: clamp01(code_hits / 4.0),
            code_intent: if has_code_intent { 1.0 } else { 0.0 },
            explanatory: if is_explanatory { 1.0 } else { 0.0 },
            multi_step: multi_step_signal,
            reasoning: clamp01(reasoning_hits / 3.5),
            system_length: clamp01(system.chars().count() as f32 / 800.0),
            trivial: clamp01(trivial_hits / 2.0),
            short_question,
            tools: if request.requires_tools() { 1.0 } else { 0.0 },
        }
    }

    /// The feature vector, in a fixed order matching [`V2_WEIGHTS`].
    pub fn as_vector(&self) -> [f32; 12] {
        [
            self.length,
            self.turns,
            self.code_block,
            self.code_terms,
            self.code_intent,
            self.explanatory,
            self.multi_step,
            self.reasoning,
            self.system_length,
            self.trivial,
            self.short_question,
            self.tools,
        ]
    }

    /// Human-readable reasons for the score, strongest first.
    ///
    /// Surfaced in the request log so a customer asking "why was this downgraded?" gets a
    /// real answer rather than "the model decided".
    pub fn explain(&self) -> Vec<&'static str> {
        let mut reasons = Vec::new();
        if self.tools > 0.0 {
            reasons.push("tool calling requested");
        }
        if self.code_block > 0.0 {
            reasons.push("contains a code block");
        }
        if self.code_intent > 0.0 {
            reasons.push("asks for code to be written or changed");
        }
        if self.reasoning > 0.5 {
            reasons.push("asks for analysis or reasoning");
        }
        if self.explanatory > 0.0 {
            reasons.push("asks for explanation or comparison");
        }
        if self.multi_step > 0.0 {
            reasons.push("several tasks chained in one request");
        }
        if self.length > 0.5 {
            reasons.push("long input context");
        }
        if self.turns > 0.5 {
            reasons.push("deep conversation history");
        }
        if self.short_question > 0.0 {
            reasons.push("short single question");
        }
        if self.trivial > 0.0 {
            reasons.push("mechanical task vocabulary");
        }
        reasons
    }
}

fn clamp01(value: f32) -> f32 {
    value.clamp(0.0, 1.0)
}

/// Which scoring model to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ClassifierVersion {
    /// Hand-tuned heuristics (Phase 3).
    #[default]
    V1,
    /// Linear model over the same features (Phase 5).
    V2,
}

/// Weights for [`ClassifierVersion::V2`], in [`Features::as_vector`] order, plus a bias.
///
/// Fitted offline by `scripts/train_classifier.py` against the labelled fixture set,
/// using feature vectors dumped from this very module (`cargo run --bin dump_features`)
/// so the model can never be trained on features that differ from the ones served.
///
/// The fit is sign-constrained: every weight is forced to the direction the domain
/// requires, because an unconstrained fit on fewer than a hundred correlated examples
/// learns spurious relationships that score well here and route badly in production.
///
/// Committed rather than loaded at runtime: scoring stays deterministic, reviewable in a
/// diff, and free of a model-loading failure mode in the hot path.
pub const V2_WEIGHTS: [f32; 12] = [
    0.0072,  // length
    0.0860,  // turns
    0.0545,  // code_block
    0.0172,  // code_terms
    0.0666,  // code_intent
    0.0843,  // explanatory
    0.1958,  // multi_step
    0.2144,  // reasoning
    0.0000,  // system_length
    -0.1189, // trivial
    -0.1998, // short_question
    0.0000,  // tools (short-circuited before scoring; retained for vector alignment)
];

/// Bias term for the V2 model.
pub const V2_BIAS: f32 = 0.4008;

/// The result of classifying one request.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Classification {
    /// Raw score in `0.0..=1.0`.
    pub score: f32,
    /// Banded complexity.
    pub complexity: Complexity,
    /// The features the score was derived from.
    pub features: Features,
    /// Which scorer produced it.
    pub version: ClassifierVersion,
    /// What kind of task this appears to be — used for provider preference.
    pub domain: TaskDomain,
}

impl Classification {
    /// Plain-language reasons this request scored the way it did.
    ///
    /// The customer-facing half of a routing decision. `Features::explain` has produced
    /// these since the classifier was written and had no caller outside its own test, so
    /// the only explanation a customer could see was a six-value enum and a bare score —
    /// which cannot answer "why was my request downgraded". Found in the enterprise
    /// readiness audit.
    pub fn explain(&self) -> Vec<String> {
        let mut reasons: Vec<String> = self
            .features
            .explain()
            .into_iter()
            .map(|r| r.to_string())
            .collect();
        reasons.insert(
            0,
            format!(
                "classified {} (score {:.2})",
                self.complexity.as_str(),
                self.score
            ),
        );
        reasons
    }
}

/// The complexity classifier.
#[derive(Debug, Clone, Copy, Default)]
pub struct Classifier {
    version: ClassifierVersion,
}

impl Classifier {
    /// A classifier using the default (V1) scorer.
    pub fn new() -> Classifier {
        Classifier::default()
    }

    /// A classifier pinned to a specific version.
    pub fn with_version(version: ClassifierVersion) -> Classifier {
        Classifier { version }
    }

    /// Which scorer this classifier uses.
    pub fn version(&self) -> ClassifierVersion {
        self.version
    }

    /// Classify a request.
    pub fn classify(&self, request: &NormalizedRequest) -> Classification {
        let features = Features::extract(request);
        let score = match self.version {
            ClassifierVersion::V1 => score_v1(&features),
            ClassifierVersion::V2 => score_v2(&features),
        };
        let domain = detect_domain(&features);
        Classification {
            score,
            complexity: Complexity::from_score(score),
            features,
            version: self.version,
            domain,
        }
    }
}

/// Heuristic scorer.
///
/// The weights encode the asymmetry described in the module docs: signals that indicate
/// difficulty (code, reasoning, length, tools) push up strongly, while signals that
/// indicate triviality pull down gently. A false "simple" costs a customer their answer
/// quality; a false "complex" costs us a few cents of margin.
pub fn score_v1(features: &Features) -> f32 {
    // Tool calling sets a floor rather than adding a term. A request that needs to call
    // functions correctly is never a job for the cheapest tier, however short it looks.
    if features.tools > 0.0 {
        return 0.75;
    }

    let mut score = 0.10; // baseline: assume some substance until shown otherwise

    score += 0.30 * features.length;
    score += 0.12 * features.turns;
    score += 0.20 * features.code_block;
    score += 0.18 * features.code_terms;
    score += 0.22 * features.code_intent;
    score += 0.28 * features.explanatory;
    score += 0.20 * features.multi_step;
    score += 0.48 * features.reasoning;
    score += 0.10 * features.system_length;

    score -= 0.18 * features.trivial;
    score -= 0.16 * features.short_question;

    clamp01(score)
}

/// Detect the task domain from extracted features.
///
/// Uses the same feature signals already extracted — no extra scan of the text.
/// Domain detection is intentionally lenient: a mixed request (e.g. "explain and
/// implement a sorting algorithm") falls to Code when code intent is present, because
/// code generation is the harder half and the one that benefits most from a
/// specialised provider.
fn detect_domain(f: &Features) -> TaskDomain {
    // Code: any explicit production intent, or heavy code vocabulary.
    if f.code_intent > 0.0 || f.code_block > 0.0 || f.code_terms >= 0.5 {
        return TaskDomain::Code;
    }
    // Reasoning: multi-step analysis or high reasoning-verb density.
    if f.reasoning >= 0.5 || f.multi_step >= 0.6 {
        return TaskDomain::Reasoning;
    }
    // Language: trivial mechanical tasks (translation, rephrasing, etc.).
    if f.trivial > 0.0 {
        return TaskDomain::Language;
    }
    TaskDomain::General
}

/// Linear scorer over the same features.
pub fn score_v2(features: &Features) -> f32 {
    if features.tools > 0.0 {
        return 0.75;
    }
    let vector = features.as_vector();
    let sum: f32 = vector
        .iter()
        .zip(V2_WEIGHTS.iter())
        .map(|(f, w)| f * w)
        .sum();
    clamp01(sum + V2_BIAS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Message, Role};
    use serde::Deserialize;

    /// One labelled example from the fixture set.
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

    fn load_cases() -> Vec<Case> {
        let raw = include_str!("../../fixtures/classifier_cases.json");
        serde_json::from_str(raw).expect("fixture file must parse")
    }

    fn build_request(case: &Case) -> NormalizedRequest {
        let mut messages = Vec::new();
        if !case.system.is_empty() {
            messages.push(Message::text(Role::System, case.system.clone()));
        }
        for (i, turn) in case.history.iter().enumerate() {
            let role = if i % 2 == 0 {
                Role::User
            } else {
                Role::Assistant
            };
            messages.push(Message::text(role, turn.clone()));
        }
        messages.push(Message::text(Role::User, case.prompt.clone()));

        NormalizedRequest {
            messages,
            tools: if case.tools {
                vec![serde_json::json!({"type": "function", "function": {"name": "f"}})]
            } else {
                vec![]
            },
            ..NormalizedRequest::simple("gpt-4o", "")
        }
    }

    fn accuracy(version: ClassifierVersion) -> (f64, Vec<String>) {
        let classifier = Classifier::with_version(version);
        let cases = load_cases();
        let mut correct = 0;
        let mut misses = Vec::new();

        for case in &cases {
            let result = classifier.classify(&build_request(case));
            if result.complexity.as_str() == case.expected {
                correct += 1;
            } else {
                misses.push(format!(
                    "{}: expected {}, got {} (score {:.3})",
                    case.name,
                    case.expected,
                    result.complexity.as_str(),
                    result.score
                ));
            }
        }
        (correct as f64 / cases.len() as f64 * 100.0, misses)
    }

    #[test]
    fn fixture_set_matches_the_specified_composition() {
        // Part 5 P3.1 calls for 100 labelled cases: 40 simple, 35 medium, 25 complex.
        let cases = load_cases();
        assert_eq!(cases.len(), 100, "expected 100 labelled cases");
        let count = |label: &str| cases.iter().filter(|c| c.expected == label).count();
        assert_eq!(count("simple"), 40);
        assert_eq!(count("medium"), 35);
        assert_eq!(count("complex"), 25);
    }

    #[test]
    fn classifier_accuracy_meets_bar() {
        // The Phase 3 acceptance criterion.
        let (accuracy, misses) = accuracy(ClassifierVersion::V1);
        assert!(
            accuracy >= 85.0,
            "V1 accuracy {accuracy:.1}% is below the 85% bar. Misclassified:\n  {}",
            misses.join("\n  ")
        );
    }

    #[test]
    fn classifier_v2_beats_v1() {
        // The Phase 5 acceptance criterion: the trained model must earn its place.
        let (v1, _) = accuracy(ClassifierVersion::V1);
        let (v2, misses) = accuracy(ClassifierVersion::V2);
        assert!(
            v2 >= v1,
            "V2 ({v2:.1}%) regressed against V1 ({v1:.1}%). Misclassified:\n  {}",
            misses.join("\n  ")
        );
        assert!(v2 >= 85.0, "V2 accuracy {v2:.1}% below bar");
    }

    #[test]
    fn no_complex_request_is_ever_classified_simple() {
        // The expensive mistake. A complex request downgraded to the cheapest tier is the
        // quality failure that loses a customer, so it must not happen even once —
        // a stricter bar than overall accuracy.
        for version in [ClassifierVersion::V1, ClassifierVersion::V2] {
            let classifier = Classifier::with_version(version);
            for case in load_cases().iter().filter(|c| c.expected == "complex") {
                let result = classifier.classify(&build_request(case));
                assert_ne!(
                    result.complexity,
                    Complexity::Simple,
                    "{:?} classified complex case {:?} as simple (score {:.3})",
                    version,
                    case.name,
                    result.score
                );
            }
        }
    }

    #[test]
    fn tool_requests_never_route_cheap() {
        // Part 5 [6a]: tool use sets a complexity floor of 0.7.
        let mut request = NormalizedRequest::simple("gpt-4o", "hi");
        request.tools = vec![serde_json::json!({"type": "function", "function": {"name": "f"}})];
        for version in [ClassifierVersion::V1, ClassifierVersion::V2] {
            let result = Classifier::with_version(version).classify(&request);
            assert!(
                result.score >= 0.7,
                "{version:?} scored a tool request {}",
                result.score
            );
            assert_eq!(result.complexity, Complexity::Complex);
        }
    }

    #[test]
    fn trivial_questions_score_simple() {
        for prompt in [
            "What is 2+2?",
            "What is the capital of France?",
            "Translate hello into Spanish",
            "Convert 10 km to miles",
        ] {
            let result = Classifier::new().classify(&NormalizedRequest::simple("gpt-4o", prompt));
            assert_eq!(
                result.complexity,
                Complexity::Simple,
                "{prompt:?} scored {:.3}",
                result.score
            );
        }
    }

    #[test]
    fn heavy_reasoning_requests_score_complex() {
        for prompt in [
            "Analyze this stack trace, diagnose the root cause, and architect a fix that \
             avoids the same class of bug across the codebase.",
            "Derive the time complexity of this algorithm step by step and prove the bound \
             is tight.",
        ] {
            let result = Classifier::new().classify(&NormalizedRequest::simple("gpt-4o", prompt));
            assert_eq!(
                result.complexity,
                Complexity::Complex,
                "{prompt:?} scored {:.3}",
                result.score
            );
        }
    }

    #[test]
    fn long_context_raises_complexity() {
        let short = Classifier::new().classify(&NormalizedRequest::simple("gpt-4o", "summarize"));
        let long = Classifier::new().classify(&NormalizedRequest::simple(
            "gpt-4o",
            &"context ".repeat(2_000),
        ));
        assert!(
            long.score > short.score,
            "long context ({:.3}) did not outscore short ({:.3})",
            long.score,
            short.score
        );
    }

    #[test]
    fn scores_are_always_in_range() {
        // Including adversarial inputs: everything must land in 0..=1.
        let inputs = vec![
            NormalizedRequest::simple("gpt-4o", ""),
            NormalizedRequest::simple("gpt-4o", &"analyze debug prove derive ".repeat(500)),
            NormalizedRequest::simple("gpt-4o", &"translate ".repeat(500)),
            NormalizedRequest {
                messages: vec![],
                ..NormalizedRequest::simple("gpt-4o", "")
            },
        ];
        for version in [ClassifierVersion::V1, ClassifierVersion::V2] {
            for request in &inputs {
                let score = Classifier::with_version(version).classify(request).score;
                assert!((0.0..=1.0).contains(&score), "{version:?} produced {score}");
                assert!(score.is_finite());
            }
        }
    }

    #[test]
    fn classification_is_deterministic() {
        // The same request must always route the same way, or a usage record cannot
        // explain a routing decision after the fact.
        let request = NormalizedRequest::simple("gpt-4o", "Refactor this module for clarity");
        let first = Classifier::new().classify(&request);
        for _ in 0..20 {
            assert_eq!(Classifier::new().classify(&request).score, first.score);
        }
    }

    #[test]
    fn features_are_normalised() {
        let request = NormalizedRequest::simple("gpt-4o", &"word ".repeat(10_000));
        let features = Features::extract(&request);
        for value in features.as_vector() {
            assert!(
                (0.0..=1.0).contains(&value),
                "feature out of range: {value}"
            );
        }
    }

    #[test]
    fn explanations_name_the_actual_signals() {
        let mut request = NormalizedRequest::simple(
            "gpt-4o",
            "Analyze this and debug the root cause:\n```rust\nfn main() {}\n```",
        );
        request.tools = vec![serde_json::json!({"type": "function", "function": {"name": "f"}})];

        let explanation = Features::extract(&request).explain();
        assert!(explanation.contains(&"tool calling requested"));
        assert!(explanation.contains(&"contains a code block"));
        assert!(explanation.contains(&"asks for analysis or reasoning"));
    }

    #[test]
    fn empty_request_does_not_panic_and_scores_low() {
        let empty = NormalizedRequest {
            messages: vec![],
            ..NormalizedRequest::simple("gpt-4o", "")
        };
        let result = Classifier::new().classify(&empty);
        assert!(
            result.score < 0.35,
            "empty request scored {:.3}",
            result.score
        );
    }
}
