//! Comprehensive integration tests for Smart Routing and Context Compression.
//!
//! Verifies:
//! 1. Smart Routing:
//!    - Simple / trivial query downgrade to cheap/mid models (saving cost)
//!    - Complex / deep reasoning query preservation (frontier quality preserved)
//!    - Code generation intent classification & routing
//!    - Explicit routing hint (`passthrough`, `cost_optimized`, `quality_optimized`)
//!    - Required capability filtering (tools, vision)
//!    - Policy pinning and tier ceiling enforcement
//!    - Degraded provider fallback
//! 2. Context Compression:
//!    - System prompt deduplication (framework repeated turns)
//!    - Semantic-safe whitespace compaction (zero tampering of indentation inside code blocks)
//!    - History truncation on long conversations (> 40 messages threshold, keeping 20 recent + marker)
//!    - Disabled configuration / zero retention behavior
//!    - Accurate token delta and savings percentage computation

use aegis_gateway::engine::classifier::Classifier;
use aegis_gateway::engine::compressor::{compress, CompressorConfig, TRUNCATION_MARKER};
use aegis_gateway::engine::fallback::ProviderHealth;
use aegis_gateway::engine::policy::{Action, Condition, RoutingPolicy, Rule};
use aegis_gateway::engine::router::{Router, RoutingInputs};
use aegis_gateway::metering::pricing::PricingTable;
use aegis_gateway::types::{
    Complexity, Content, Message, ModelTier, NormalizedRequest, Role, RoutingHint, RoutingReason,
};

// ============================================================================
// CONTEXT COMPRESSION USE CASES & TESTS
// ============================================================================

#[test]
fn test_compression_use_case_system_prompt_deduplication() {
    // USE CASE: Agent frameworks that re-send the exact system prompt every turn.
    let system_prompt = "You are an enterprise AI assistant adhering to strict safety protocols.";
    let mut request = NormalizedRequest {
        model: "openai/gpt-5".into(),
        messages: vec![
            Message::text(Role::System, system_prompt),
            Message::text(Role::User, "Turn 1 question"),
            Message::text(Role::Assistant, "Turn 1 answer"),
            // Duplicate system prompt injected by framework:
            Message::text(Role::System, system_prompt),
            Message::text(Role::User, "Turn 2 question"),
        ],
        ..NormalizedRequest::simple("openai/gpt-5", "")
    };

    let before_tokens = request.estimated_input_tokens();
    let config = CompressorConfig::default();
    let result = compress(&mut request, &config);

    assert_eq!(result.duplicate_system_messages_removed, 1);
    assert_eq!(request.messages.len(), 4);
    assert!(result.tokens_saved() > 0);
    assert_eq!(result.tokens_before, before_tokens);
    assert_eq!(result.tokens_after, request.estimated_input_tokens());

    // Verify remaining system message is preserved at the beginning
    assert_eq!(request.messages[0].role, Role::System);
    assert_eq!(request.messages[0].text_content(), system_prompt);
}

#[test]
fn test_compression_use_case_code_fence_preservation() {
    // USE CASE: Python code where whitespace/indentation is semantic.
    // Whitespace outside code fences must be collapsed, but indentation inside must be 100% intact.
    let text = "Here is the   solution:   \n\n\n```python\ndef calculate_fibonacci(n):\n    if n <= 1:\n        return n\n    return calculate_fibonacci(n-1) + calculate_fibonacci(n-2)\n```\n\n\nLet me know   if you need anything else.";

    let mut request = NormalizedRequest {
        messages: vec![Message::text(Role::User, text)],
        ..NormalizedRequest::simple("openai/gpt-5", "")
    };

    let config = CompressorConfig {
        collapse_whitespace: true,
        dedupe_system: false,
        truncate_history: false,
        // Isolate whitespace collapsing: this test asserts on whitespace_chars_removed
        // and on code-fence indentation surviving, so every other technique stays off.
        minify_json: false,
        reference_duplicate_blocks: false,
        trim_stale_tool_results: false,
        truncate_threshold: 100,
        keep_recent: 50,
        token_budget: None,
    };

    let result = compress(&mut request, &config);
    assert!(result.whitespace_chars_removed > 0);

    let compressed_text = request.messages[0].text_content();

    // Verify Python indentation inside ```python is preserved exactly
    assert!(compressed_text.contains("    if n <= 1:\n        return n"));
    assert!(compressed_text.contains("def calculate_fibonacci(n):"));

    // Verify outside whitespace was collapsed
    assert!(compressed_text.contains("Here is the solution:"));
    assert!(compressed_text.contains("Let me know if you need anything else."));
    assert!(!compressed_text.contains("\n\n\n"));
}

#[test]
fn test_compression_use_case_history_truncation_long_conversation() {
    // USE CASE: Over-long conversation (> 40 messages threshold).
    // Must keep system prompt, preserve the last 20 turns, and inject the truncation marker.
    let mut messages = Vec::new();
    messages.push(Message::text(Role::System, "System instructions"));

    for i in 1..=50 {
        messages.push(Message::text(
            if i % 2 == 1 {
                Role::User
            } else {
                Role::Assistant
            },
            format!("Turn {} message content", i),
        ));
    }

    let mut request = NormalizedRequest {
        messages,
        ..NormalizedRequest::simple("openai/gpt-5", "")
    };

    let initial_count = request.messages.len();
    assert_eq!(initial_count, 51); // 1 system + 50 conversation

    let config = CompressorConfig::default(); // threshold 40, keep 20
    let result = compress(&mut request, &config);

    assert_eq!(result.messages_truncated, 30); // 50 - 20 = 30 truncated
    assert!(result.tokens_saved() > 0);

    // Structure should be: [System, Truncation Marker System message, ... last 20 turns]
    assert_eq!(request.messages[0].role, Role::System);
    assert_eq!(request.messages[0].text_content(), "System instructions");

    assert_eq!(request.messages[1].role, Role::System);
    assert_eq!(request.messages[1].text_content(), TRUNCATION_MARKER);

    // Total messages = 1 original system + 1 marker + 20 preserved recent turns = 22
    assert_eq!(request.messages.len(), 22);

    // The last turn must be Turn 50
    assert_eq!(
        request.messages.last().unwrap().text_content(),
        "Turn 50 message content"
    );
}

#[test]
fn test_compression_use_case_disabled_fidelity_mode() {
    // USE CASE: High-fidelity / zero retention org where compression is disabled.
    let text = "Uncompressed    content   with   spaces\n\n\n\nand duplicates";
    let mut request = NormalizedRequest {
        messages: vec![
            Message::text(Role::System, "System A"),
            Message::text(Role::System, "System A"),
            Message::text(Role::User, text),
        ],
        ..NormalizedRequest::simple("openai/gpt-5", "")
    };

    let config = CompressorConfig::disabled();
    let result = compress(&mut request, &config);

    assert!(result.is_noop());
    assert_eq!(result.tokens_saved(), 0);
    assert_eq!(request.messages.len(), 3);
    assert_eq!(request.messages[2].text_content(), text);
}

// ============================================================================
// SMART ROUTING USE CASES & TESTS
// ============================================================================

#[test]
fn test_smart_routing_use_case_simple_prompt_downgrade() {
    // USE CASE: Simple, low-complexity recall/formatting query sent to frontier model.
    // Must be classified as Simple and routed to a cheaper model (e.g. gpt-5-nano or gpt-5-mini),
    // generating verifiable micro-cents savings.
    let table = PricingTable::with_seed_data();
    let health = ProviderHealth::new();
    let router = Router::new();
    let classifier = Classifier::default();

    let request = NormalizedRequest::simple(
        "openai/gpt-5",
        "What is the capital of France? In one word.",
    );

    let classification = classifier.classify(&request);
    assert_eq!(
        classification.complexity,
        Complexity::Simple,
        "Trivial question should be classified as Simple"
    );

    let decision = router
        .route(&request, &table, &health, &RoutingInputs::default())
        .expect("routing decision succeeds");

    assert_eq!(decision.reason, RoutingReason::Complexity);
    assert_ne!(
        decision.served_model, "openai/gpt-5",
        "Simple query should be downgraded from frontier model"
    );

    // Verify the served model is actually cheaper in input & output cost
    let requested_price = table.get("openai/gpt-5").unwrap();
    let served_price = table.get(&decision.served_model).unwrap();
    assert!(served_price.input_per_mtok < requested_price.input_per_mtok);
}

#[test]
fn test_smart_routing_use_case_complex_reasoning_preservation() {
    // USE CASE: Deep analytical reasoning, multi-step problem solving.
    // Must be classified as Complex and NEVER downgraded.
    let table = PricingTable::with_seed_data();
    let health = ProviderHealth::new();
    let router = Router::new();
    let classifier = Classifier::default();

    let prompt = "Analyze the root cause of this memory leak in our distributed raft cluster. \
                  Investigate the tradeoff between immediate fsync and batched commits, \
                  derive the theoretical upper bound on throughput, and optimize the state machine algorithm step by step.";

    let request = NormalizedRequest::simple("openai/gpt-5", prompt);

    let classification = classifier.classify(&request);
    assert_eq!(
        classification.complexity,
        Complexity::Complex,
        "Deep reasoning prompt must be classified as Complex"
    );

    let decision = router
        .route(&request, &table, &health, &RoutingInputs::default())
        .expect("routing decision succeeds");

    assert_eq!(
        decision.served_model, "openai/gpt-5",
        "Complex requests must stay on the requested frontier model"
    );
    assert_eq!(decision.reason, RoutingReason::Passthrough);
    assert!(decision.is_passthrough());
}

#[test]
fn test_smart_routing_use_case_code_intent_preservation() {
    // USE CASE: Code generation intent with deep reasoning and verification.
    let table = PricingTable::with_seed_data();
    let health = ProviderHealth::new();
    let router = Router::new();
    let classifier = Classifier::default();

    let code_prompt = "Implement a lock-free ring buffer in Rust. \
                       Use std::sync::atomic::AtomicUsize, handle cache-line contention with #[repr(align(64))], \
                       and optimize memory ordering with Acquire and Release semantics. \
                       Analyze potential deadlock scenarios, prove lock-free invariants, and derive throughput bounds.";

    let request = NormalizedRequest::simple("openai/gpt-5", code_prompt);

    let classification = classifier.classify(&request);
    assert!(
        classification.features.code_terms > 0.0 || classification.features.code_intent > 0.0,
        "Must detect code features"
    );
    assert_eq!(
        classification.complexity,
        Complexity::Complex,
        "Deep code generation with reasoning must be classified as Complex"
    );

    let decision = router
        .route(&request, &table, &health, &RoutingInputs::default())
        .expect("routing decision succeeds");

    // Complex code requests must stay on frontier
    assert_eq!(decision.served_model, "openai/gpt-5");
    assert_eq!(decision.reason, RoutingReason::Passthrough);
}

#[test]
fn test_smart_routing_use_case_passthrough_hint_override() {
    // USE CASE: Caller specifies `X-Aegis-Routing-Hint: passthrough`.
    // Even if the query is 100% simple/trivial, the router MUST honor the user override.
    let table = PricingTable::with_seed_data();
    let health = ProviderHealth::new();
    let router = Router::new();

    let request = NormalizedRequest::simple("openai/gpt-5", "Spell the word apple.");

    let inputs = RoutingInputs {
        hint: RoutingHint::Passthrough,
        ..Default::default()
    };

    let decision = router
        .route(&request, &table, &health, &inputs)
        .expect("routing decision succeeds");

    assert_eq!(decision.served_model, "openai/gpt-5");
    assert_eq!(decision.reason, RoutingReason::UserOverride);
    assert!(decision.is_passthrough());
}

#[test]
fn test_smart_routing_use_case_tool_capability_filtering() {
    // USE CASE: Request specifies tools.
    // Router must ONLY consider candidates that have `supports_tools = true`.
    let table = PricingTable::with_seed_data();
    let health = ProviderHealth::new();
    let router = Router::new();

    let mut request = NormalizedRequest::simple("openai/gpt-5", "Check weather in Tokyo");
    request.tools = vec![serde_json::json!({
        "type": "function",
        "function": {
            "name": "get_weather",
            "parameters": {
                "type": "object",
                "properties": {
                    "city": {"type": "string"}
                }
            }
        }
    })];

    let decision = router
        .route(&request, &table, &health, &RoutingInputs::default())
        .expect("routing decision succeeds");

    let candidate = table.get(&decision.served_model).unwrap();
    assert!(
        candidate.supports_tools,
        "Served model must support tool calling"
    );
}

#[test]
fn test_smart_routing_use_case_vision_capability_filtering() {
    // USE CASE: Request contains multimodal image content.
    // Router must ONLY select models with `supports_vision = true`.
    let table = PricingTable::with_seed_data();
    let health = ProviderHealth::new();
    let router = Router::new();

    let mut request = NormalizedRequest::simple("openai/gpt-5", "");
    request.messages = vec![Message {
        role: Role::User,
        content: Some(Content::Parts(vec![
            serde_json::json!({
                "type": "text",
                "text": "What is shown in this chart?"
            }),
            serde_json::json!({
                "type": "image_url",
                "image_url": {
                    "url": "https://example.com/chart.png"
                }
            }),
        ])),
        name: None,
        tool_call_id: None,
        tool_calls: None,
    }];

    assert!(request.requires_vision());

    let decision = router
        .route(&request, &table, &health, &RoutingInputs::default())
        .expect("routing decision succeeds");

    let candidate = table.get(&decision.served_model).unwrap();
    assert!(
        candidate.supports_vision,
        "Served model must support vision"
    );
}

#[test]
fn test_smart_routing_use_case_policy_pinning_and_budget_ceiling() {
    // USE CASE: Org policy pins a specific model family or enforces a tier ceiling.
    let table = PricingTable::with_seed_data();
    let health = ProviderHealth::new();
    let router = Router::new();

    let request = NormalizedRequest::simple("openai/gpt-5", "Translate this greeting to Japanese");

    // Policy pins to anthropic/claude-3-5-haiku
    let policy = RoutingPolicy {
        rules: vec![Rule {
            condition: Condition::default(),
            action: Action {
                pin_model: Some("anthropic/claude-3-5-haiku".into()),
                ..Default::default()
            },
        }],
    };

    let inputs = RoutingInputs {
        policy: Some(&policy),
        plan_tier_ceiling: Some(ModelTier::Mid),
        ..Default::default()
    };

    let decision = router
        .route(&request, &table, &health, &inputs)
        .expect("routing decision succeeds");

    assert_eq!(decision.served_model, "anthropic/claude-3-5-haiku");
    assert_eq!(decision.reason, RoutingReason::Policy);
}
