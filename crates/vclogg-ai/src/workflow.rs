//! Per-run workflow policy and bounded investigation plans.
use crate::{ToolRisk, tool_descriptor};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentMode {
    Ask,
    Plan,
    #[default]
    Execute,
}
impl AgentMode {
    pub fn permits(self, name: &str) -> bool {
        self == Self::Execute
            || tool_descriptor(name).is_some_and(|tool| tool.risk() == ToolRisk::Observe)
    }
    pub(crate) fn instructions(self) -> &'static str {
        match self {
            Self::Ask => {
                "\n工作模式：问答。可读取证据和回答问题；禁止改变文件、标记、视图和记忆，也不调用 Shell 或外部 MCP。\n"
            }
            Self::Plan => {
                "\n工作模式：计划。只读调查，使用 update_plan 维护步骤，明确证据、验证方法和待确认项。最后交付计划；不得执行修改。用户须切换到执行模式并发送请求后才能执行。\n"
            }
            Self::Execute => {
                "\n工作模式：执行。复杂任务可用 update_plan 跟踪步骤，用 delegate_analysis 获取独立证据分析。遵循现有工具权限；模式不替代操作确认。\n"
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Pending,
    InProgress,
    Complete,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct PlanStep {
    pub text: String,
    pub status: StepStatus,
}

pub(crate) fn parse_plan(value: &serde_json::Value) -> anyhow::Result<Vec<PlanStep>> {
    let steps: Vec<PlanStep> = serde_json::from_value(value["steps"].clone())?;
    anyhow::ensure!(
        !steps.is_empty() && steps.len() <= 12,
        "Use 1–12 plan steps"
    );
    anyhow::ensure!(
        steps
            .iter()
            .all(|s| !s.text.trim().is_empty() && s.text.len() <= 1024),
        "Plan step text must be nonempty and bounded"
    );
    anyhow::ensure!(
        steps
            .iter()
            .filter(|s| s.status == StepStatus::InProgress)
            .count()
            <= 1,
        "Only one plan step may be in progress"
    );
    Ok(steps)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn readonly_modes_reject_side_effects_and_unknown_tools() {
        for mode in [AgentMode::Ask, AgentMode::Plan] {
            for name in ["shell", "set_marks", "navigate", "save_memory", "unknown"] {
                assert!(!mode.permits(name), "{name}");
            }
            for name in [
                "get_context",
                "search_logs",
                "update_plan",
                "delegate_analysis",
            ] {
                assert!(mode.permits(name), "{name}");
            }
        }
    }
    #[test]
    fn plans_reject_multiple_active_steps() {
        let value = serde_json::json!({"steps":[{"text":"Read","status":"in_progress"},{"text":"Check","status":"in_progress"}]});
        assert!(parse_plan(&value).is_err());
    }
}
