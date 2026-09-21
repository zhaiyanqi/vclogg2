//! Conservative, provider-independent context estimate for display only.
use crate::tools::ToolDefinition;
use crate::{AgentMessage, ContextUsage, ProviderConfig};

pub fn estimate_tokens(text: &str) -> u32 {
    let mut ascii = 0u32;
    let mut non_ascii = 0u32;
    for character in text.chars() {
        if character.is_ascii() {
            ascii = ascii.saturating_add(1);
        } else {
            non_ascii = non_ascii.saturating_add(1);
        }
    }
    ascii.div_ceil(4).saturating_add(non_ascii)
}

pub(crate) fn usage(
    config: &ProviderConfig,
    system: &str,
    prompt: &str,
    skill_summaries: &str,
    messages: &[AgentMessage],
    tools: &[ToolDefinition],
) -> ContextUsage {
    let prompt_tokens = estimate_tokens(prompt);
    let skill_tokens = estimate_tokens(skill_summaries);
    let full_system = estimate_tokens(system);
    let tool_tokens = if !tools.is_empty() {
        let wire = tools
            .iter()
            .map(|tool| format!("{} {} {}", tool.name, tool.description, tool.parameters))
            .collect::<Vec<_>>()
            .join("\n");
        estimate_tokens(&wire)
    } else {
        0
    };
    let conversation_tokens = serde_json::to_string(messages)
        .ok()
        .map_or(0, |text| estimate_tokens(&text));
    ContextUsage {
        system_tokens: full_system.saturating_sub(prompt_tokens.saturating_add(skill_tokens)),
        prompt_tokens,
        skill_tokens,
        tool_tokens,
        conversation_tokens,
        context_window_tokens: config.context_window_tokens,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn estimates_unicode_and_percent_without_fake_capacity() {
        assert!(estimate_tokens("错误") >= 2);
        let usage = ContextUsage {
            conversation_tokens: 250,
            context_window_tokens: Some(1000),
            ..Default::default()
        };
        assert_eq!(usage.percent(), Some(25));
        assert_eq!(ContextUsage::default().percent(), None);
    }
}
