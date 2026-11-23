//! `helpTexts`, the prompt guards (`prompt-utils.ts`) and the channel length
//! instruction (`ai-output-length.ts`).

use chatbotx_core::flow_config::channel_rules::{
    SendTextLengthProps, resolve_send_text_length_limits,
};

use super::toolset::Toolset;

pub const FILE_SEARCH_NO_RESULT: &str = "No files matching the search query were found.";
pub const FILE_SEARCH_DESCRIPTION: &str = "Search the workspace's uploaded knowledge files for factual, private, or domain-specific information needed to answer the user. Use this when the answer may depend on uploaded files. Do NOT use for greetings, small talk, or casual salutations.";
pub const FILE_SEARCH_QUERY_DESCRIPTION: &str = "Search keywords to find relevant information";
pub const FALLBACK_LOOKUP: &str = "I've found some data, but I couldn't generate a complete answer yet. Could you please specify what you're looking for (price, size, color)?";

pub fn file_search_found_prefix(count: usize) -> String {
    format!("Found {count} matching file(s):")
}

const TOOL_OUTPUT_GUARD: &str = "IMPORTANT RULES (REQUIRED):\n- Never send raw JSON or tool outputs directly to the user.\n- If you use a tool, summarize the result concisely in natural language.\n- Only provide the most relevant information the customer is asking for.";
pub(crate) const FABRICATION_GUARD: &str = "TOOL DATA RULES (REQUIRED):\n- Never fabricate, guess, or invent data — especially product names, prices, specifications, availability, or image URLs.\n- Only include image URLs that appear exactly in tool results. Never generate, guess, or modify image URLs.\n- Only include information that appears explicitly in tool results. If a tool returns no results, tell the user you could not find the information.";
const KNOWLEDGE_BASE_GUARD: &str = "KNOWLEDGE BASE RULES (REQUIRED):\n- You MUST call search_knowledge_base BEFORE answering any question about products, services, prices, images, or company-specific details.\n- Do NOT use search_knowledge_base for greetings, small talk, or simple clarification.\n- If search_knowledge_base returns no results or insufficient information, tell the user you could not find matching information.";

pub mod summarizer {
    pub fn previous_summary(summary: &str) -> String {
        format!("This is the previous summary of the conversation: \"{summary}\"")
    }
    pub const LATEST_MESSAGES: &str = "Here are the latest messages:";
    pub const UPDATE_SUMMARY_PROMPT: &str = "Please update the previous summary by incorporating the new information. Keep the summary concise and succinct (under 1000 characters), focusing on key information such as: customer name, issues encountered, needs, order status, and agreed decisions. Return the new summary.";
    pub const CONVERSATION_HISTORY: &str = "Below is the conversation history:";
    pub const NEW_SUMMARY_PROMPT: &str = "Please summarize the above conversation concisely and succinctly (under 1000 characters). Focus on key information such as: customer name, issues encountered, needs, order status, and agreed decisions. Return the summary.";
    pub fn shorten_prompt(summary: &str) -> String {
        format!(
            "The following summary is too long: \"{summary}\". Please shorten it to under 1000 characters while still retaining the most important key points."
        )
    }
}

/// `appendToolOutputGuard`, then `appendFabricationGuard` and
/// `appendKnowledgeBaseGuard` for the loaded tools.
pub fn guarded_prompt(prompt: &str, tools: &Toolset) -> String {
    let mut out = format!("{prompt}\n\n{TOOL_OUTPUT_GUARD}").trim().to_owned();
    if !tools.is_empty() {
        out = format!("{out}\n\n{FABRICATION_GUARD}").trim().to_owned();
    }
    if tools.has("search_knowledge_base") {
        out = format!("{out}\n\n{KNOWLEDGE_BASE_GUARD}").trim().to_owned();
    }
    out
}

/// `resolveAIOutputCharBudget`: 80 % of the channel's text limit.
pub fn output_char_budget(channel: Option<&str>) -> usize {
    let limit = resolve_send_text_length_limits(SendTextLengthProps {
        channel,
        has_buttons: None,
        has_quick_replies: None,
    })
    .text;
    (limit as f64 * 0.8).floor() as usize
}

/// `buildChannelLengthInstruction`.
pub fn channel_length_instruction(channel: Option<&str>) -> String {
    let budget = output_char_budget(channel);
    let destination = match channel.map(str::trim).filter(|c| !c.is_empty()) {
        Some(name) => format!("one {name} chat message"),
        None => "one chat message".to_owned(),
    };
    format!(
        "Your answer will be sent as {destination}. Keep it under {budget} characters including spaces. Use plain text, no markdown headings."
    )
}

/// `withChannelLengthInstruction`: the authored system prompt, then the
/// length instruction.
pub fn with_channel_length_instruction(system: Option<&str>, channel: Option<&str>) -> String {
    let instruction = channel_length_instruction(channel);
    match system.map(str::trim).filter(|s| !s.is_empty()) {
        Some(authored) => format!("{authored}\n\n{instruction}"),
        None => instruction,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // apps/worker/__tests__/ai-output-length.test.ts
    #[test]
    fn offers_80_percent_of_the_channel_text_limit() {
        for (channel, budget) in [
            ("messenger", 1600),
            ("zalo", 1600),
            ("instagram", 800),
            ("threads", 400),
            ("whatsapp", 3276),
            ("telegram", 3276),
        ] {
            assert_eq!(output_char_budget(Some(channel)), budget, "{channel}");
        }
    }

    #[test]
    fn falls_back_to_the_omnichannel_limit() {
        assert_eq!(output_char_budget(None), 800);
        assert_eq!(output_char_budget(Some("not-a-channel")), 800);
    }

    #[test]
    fn names_the_channel_and_its_budget() {
        assert_eq!(
            channel_length_instruction(Some("messenger")),
            "Your answer will be sent as one messenger chat message. Keep it under 1600 characters including spaces. Use plain text, no markdown headings."
        );
    }

    #[test]
    fn uses_a_generic_name_when_the_channel_is_missing() {
        assert_eq!(
            channel_length_instruction(None),
            "Your answer will be sent as one chat message. Keep it under 800 characters including spaces. Use plain text, no markdown headings."
        );
    }

    #[test]
    fn appends_the_instruction_after_the_authors_system_prompt() {
        assert_eq!(
            with_channel_length_instruction(Some("  You are a stylist.  "), Some("messenger")),
            format!(
                "You are a stylist.\n\n{}",
                channel_length_instruction(Some("messenger"))
            )
        );
    }

    #[test]
    fn uses_the_instruction_alone_when_the_system_prompt_is_empty() {
        for system in [None, Some(""), Some("   ")] {
            assert_eq!(
                with_channel_length_instruction(system, Some("messenger")),
                channel_length_instruction(Some("messenger"))
            );
        }
    }
}
