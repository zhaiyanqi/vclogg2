use crate::ToolCall;
use anyhow::{Result, bail};
use serde_json::{Value, json};
use std::collections::BTreeSet;

#[derive(Clone, Debug)]
pub struct ToolDefinition {
    pub name: &'static str,
    pub description: &'static str,
    pub parameters: Value,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum ToolGroup {
    Core,
    Evidence,
    Files,
    SearchUi,
    Marks,
    SourceSearch,
    SourceSymbols,
    Memory,
    Mcp,
    Skills,
}

impl ToolGroup {
    pub(crate) const OPTIONAL: [Self; 9] = [
        Self::Evidence,
        Self::Files,
        Self::SearchUi,
        Self::Marks,
        Self::SourceSearch,
        Self::SourceSymbols,
        Self::Memory,
        Self::Mcp,
        Self::Skills,
    ];

    pub(crate) const fn id(self) -> &'static str {
        match self {
            Self::Core => "core",
            Self::Evidence => "evidence",
            Self::Files => "files",
            Self::SearchUi => "search_ui",
            Self::Marks => "marks",
            Self::SourceSearch => "source_search",
            Self::SourceSymbols => "source_symbols",
            Self::Memory => "memory",
            Self::Mcp => "mcp",
            Self::Skills => "skills",
        }
    }

    pub(crate) fn from_id(id: &str) -> Option<Self> {
        Self::OPTIONAL.into_iter().find(|group| group.id() == id)
    }
}

pub(crate) fn group_for_tool(name: &str) -> ToolGroup {
    match name {
        "load_tool_group" | "get_context" | "list_logs" | "search_logs" | "read_log_context" => {
            ToolGroup::Core
        }
        "read_logs" | "search_results" | "summarize_search" | "read_log_segment" => {
            ToolGroup::Evidence
        }
        "locate_files" | "list_log_directory" | "open_file" | "close_file" | "switch_file"
        | "reveal_file" => ToolGroup::Files,
        "show_search" | "control_search" | "append_search" | "list_filters" => ToolGroup::SearchUi,
        "list_colors" | "set_marks" | "highlight_keyword" | "text_mark" | "list_marks"
        | "navigate" => ToolGroup::Marks,
        "list_source_workspaces"
        | "rg_list_files"
        | "rg_search"
        | "rg_count"
        | "read_source"
        | "find_source_files" => ToolGroup::SourceSearch,
        "find_symbols" | "source_outline" | "locate_log_origin" | "find_definition"
        | "find_references" => ToolGroup::SourceSymbols,
        "search_memory" | "save_memory" | "delete_memory" => ToolGroup::Memory,
        "list_mcp_servers" | "list_mcp_tools" | "call_mcp_tool" => ToolGroup::Mcp,
        "list_skills" | "read_skill" => ToolGroup::Skills,
        _ => ToolGroup::Core,
    }
}

pub(crate) fn tool_definitions_for(
    loaded: &BTreeSet<ToolGroup>,
    available: &BTreeSet<ToolGroup>,
) -> Vec<ToolDefinition> {
    tool_definitions()
        .into_iter()
        .filter_map(|mut tool| {
            let group = group_for_tool(tool.name);
            let visible =
                group == ToolGroup::Core || (available.contains(&group) && loaded.contains(&group));
            if !visible {
                return None;
            }
            if tool.name == "load_tool_group" {
                tool.parameters["properties"]["group"]["enum"] = json!(
                    ToolGroup::OPTIONAL
                        .into_iter()
                        .filter(|group| available.contains(group))
                        .map(ToolGroup::id)
                        .collect::<Vec<_>>()
                );
            }
            Some(tool)
        })
        .collect()
}

pub(crate) fn groups_from_tool_history(
    names: impl IntoIterator<Item = impl AsRef<str>>,
) -> BTreeSet<ToolGroup> {
    names
        .into_iter()
        .map(|name| group_for_tool(name.as_ref()))
        .filter(|group| *group != ToolGroup::Core)
        .collect()
}

pub fn tool_definitions() -> Vec<ToolDefinition> {
    let string = || json!({"type":"string","maxLength":8192});
    let path_string = || json!({"type":"string","maxLength":1024});
    let glob_list =
        || json!({"type":"array","items":{"type":"string","maxLength":256},"maxItems":16});
    let id = || json!({"type":"integer","minimum":1});
    let boolean = || json!({"type":"boolean"});
    let choice = |values: &[&str]| json!({"type":"string","enum":values});
    let reference = || json!({"type":"object","properties":{"document_id":id(),"version":string(),"line":id()},"required":["document_id","version","line"],"additionalProperties":false});
    let make = |name, description, properties, required| ToolDefinition {
        name,
        description,
        parameters: json!({"type":"object","properties":properties,"required":required,"additionalProperties":false}),
    };
    vec![
        make(
            "load_tool_group",
            "按需加载一组工具。仅在当前工具不足时调用；同组只需加载一次。",
            json!({"group":{"type":"string","enum":ToolGroup::OPTIONAL.map(ToolGroup::id)}}),
            json!(["group"]),
        ),
        make(
            "list_source_workspaces",
            "列出已配置的只读源码工作区及数字 root ID。",
            json!({}),
            json!([]),
        ),
        make(
            "rg_list_files",
            "枚举源码文件；支持相对子目录、包含/排除 glob 和隐藏文件。不读取内容。",
            json!({"root":{"type":"integer","minimum":0},"path":path_string(),"globs":glob_list(),"include_hidden":boolean(),"offset":{"type":"integer","minimum":0},"limit":{"type":"integer","minimum":1,"maximum":200}}),
            json!(["root"]),
        ),
        make(
            "rg_search",
            "在源码工作区搜索，返回有界的路径、行列、上下文和匹配范围。默认字面量且区分大小写。",
            json!({"root":{"type":"integer","minimum":0},"query":string(),"path":path_string(),"regex":boolean(),"ignore_case":boolean(),"word":boolean(),"globs":glob_list(),"include_hidden":boolean(),"context_before":{"type":"integer","minimum":0,"maximum":5},"context_after":{"type":"integer","minimum":0,"maximum":5},"max_results":{"type":"integer","minimum":1,"maximum":200}}),
            json!(["root", "query"]),
        ),
        make(
            "rg_count",
            "按源码文件统计匹配数，不返回匹配文本；选项同 rg_search，按数量降序。",
            json!({"root":{"type":"integer","minimum":0},"query":string(),"path":path_string(),"regex":boolean(),"ignore_case":boolean(),"word":boolean(),"globs":glob_list(),"include_hidden":boolean(),"max_files":{"type":"integer","minimum":1,"maximum":200}}),
            json!(["root", "query"]),
        ),
        make(
            "read_source",
            "读取工作区内 UTF-8 源码的有界区间。path 为相对路径；默认 100 行，最多 200 行/32 KiB。",
            json!({"root":{"type":"integer","minimum":0},"path":path_string(),"start_line":id(),"limit":{"type":"integer","minimum":1,"maximum":200}}),
            json!(["root", "path"]),
        ),
        make(
            "find_source_files",
            "按不区分大小写的路径子串查找源码文件，返回有界相对路径。",
            json!({"root":{"type":"integer","minimum":0},"query":string()}),
            json!(["root", "query"]),
        ),
        make(
            "find_symbols",
            "用 Tree-sitter 按名称查找常见语言定义；结果是语法候选，不是语义引用。",
            json!({"root":{"type":"integer","minimum":0},"query":string()}),
            json!(["root", "query"]),
        ),
        make(
            "source_outline",
            "用 Tree-sitter 列出单个源码文件的定义；path 相对工作区 root。",
            json!({"root":{"type":"integer","minimum":0},"path":string()}),
            json!(["root", "path"]),
        ),
        make(
            "locate_log_origin",
            "根据已读日志行、堆栈、文件行号或 logger 线索定位源码候选；须用 read_source 验证。",
            json!({"root":{"type":"integer","minimum":0},"clue":string()}),
            json!(["root", "clue"]),
        ),
        make(
            "find_definition",
            "通过语言服务器查询 1-based 行列处的定义；不可用时回退到语法候选。",
            json!({"root":{"type":"integer","minimum":0},"path":string(),"line":id(),"column":id()}),
            json!(["root", "path", "line", "column"]),
        ),
        make(
            "find_references",
            "通过语言服务器查询 1-based 行列处的引用；不可用时回退到有界文本候选。",
            json!({"root":{"type":"integer","minimum":0},"path":string(),"line":id(),"column":id()}),
            json!(["root", "path", "line", "column"]),
        ),
        make(
            "list_mcp_servers",
            "列出用户为本次运行启用的 MCP 服务；其权限范围可能超出已捕获日志。",
            json!({}),
            json!([]),
        ),
        make(
            "list_mcp_tools",
            "连接 MCP 服务并分页列出工具名、说明和 inputSchema；调用前先读 schema。",
            json!({"server_id":string(),"offset":{"type":"integer","minimum":0}}),
            json!(["server_id"]),
        ),
        make(
            "call_mcp_tool",
            "调用已发现的 MCP 工具。arguments_json 须匹配 inputSchema；超时后先查状态，不盲目重试写操作。",
            json!({"server_id":string(),"tool_name":string(),"arguments_json":{"type":"string","maxLength":32768}}),
            json!(["server_id", "tool_name", "arguments_json"]),
        ),
        make(
            "search_memory",
            "按标题/内容子串搜索跨会话记忆；空 query 列表，next_offset 分页。记忆不是实时证据或授权。",
            json!({"query":string(),"offset":{"type":"integer","minimum":0}}),
            json!(["query"]),
        ),
        make(
            "save_memory",
            "保存简短持久偏好或事实。先查重；新增省略 id 且 revision=0，更新使用查询所得 id/revision。不得保存凭据或批量日志。",
            json!({"id":string(),"revision":{"type":"integer","minimum":0},"title":{"type":"string","maxLength":120},"content":{"type":"string","maxLength":4096}}),
            json!(["revision", "title", "content"]),
        ),
        make(
            "delete_memory",
            "仅按用户要求删除一条记忆；使用 search_memory 返回的准确 id/revision，冲突后重新读取。",
            json!({"id":string(),"revision":{"type":"integer","minimum":1}}),
            json!(["id", "revision"]),
        ),
        make(
            "get_context",
            "获取当前区域、选区、活动文件和搜索的元数据及引用，不含日志正文。",
            json!({}),
            json!([]),
        ),
        make(
            "list_logs",
            "列出允许访问的文件、路径、打开状态和引用版本；含未打开的搜索结果或附件。",
            json!({"offset":{"type":"integer","minimum":0}}),
            json!([]),
        ),
        make(
            "locate_files",
            "在捕获目录内按不区分大小写的路径子串查找文件；仅返回元数据和本次运行的 file_id。",
            json!({"query":string(),"offset":{"type":"integer","minimum":0}}),
            json!(["query"]),
        ),
        make(
            "list_log_directory",
            "列出捕获日志目录的有界树；遵守子目录、隐藏目录和文件类型过滤。path 选子树，depth 默认 3。",
            json!({"path":path_string(),"depth":{"type":"integer","minimum":1,"maximum":8},"offset":{"type":"integer","minimum":0},"limit":{"type":"integer","minimum":1,"maximum":200}}),
            json!([]),
        ),
        make(
            "open_file",
            "打开/激活日志或工作区文件。传 document_id+version、file_id，或 root+相对 path；返回新 ID/版本，不含正文。",
            json!({"document_id":id(),"version":string(),"file_id":string(),"root":{"type":"integer","minimum":0},"path":path_string()}),
            json!([]),
        ),
        make(
            "close_file",
            "关闭一个日志标签，不删除源文件。confirmation_pending 时等待用户，不得声称已关闭或重复调用。",
            json!({"document_id":id(),"version":string()}),
            json!(["document_id", "version"]),
        ),
        make(
            "switch_file",
            "按已验证 ID/版本切换打开文件，保留视口和搜索状态，不读取正文。",
            json!({"document_id":id(),"version":string()}),
            json!(["document_id", "version"]),
        ),
        make(
            "reveal_file",
            "在文件侧栏定位文件但不打开、不读取；传 document_id+version 或 file_id。",
            json!({"document_id":id(),"version":string(),"file_id":string()}),
            json!([]),
        ),
        make(
            "read_logs",
            "读取 1-based 日志区间；默认 10 行，最多 100 行/16 KiB、每行 2048 字符。长行改用 read_log_segment。",
            json!({"document_id":id(),"version":string(),"start_line":id(),"limit":{"type":"integer","minimum":1,"maximum":100}}),
            json!(["document_id", "version", "start_line"]),
        ),
        make(
            "search_logs",
            "后台搜索日志，scope=current/open/directory；返回 search_id、数量和最多 20 个引用，不含正文。字面量中 | 表示 OR；搜索管道符须用正则转义。",
            json!({"scope":choice(&["current","open","directory"]),"document_id":id(),"query":string(),"case_sensitive":boolean(),"regex":boolean()}),
            json!(["scope", "query"]),
        ),
        make(
            "search_results",
            "分页取得本次搜索的引用，不含正文；默认 20，最多 40 条/12 KiB，offset 从 0 开始。",
            json!({"search_id":string(),"offset":{"type":"integer","minimum":0},"limit":{"type":"integer","minimum":1,"maximum":40}}),
            json!(["search_id"]),
        ),
        make(
            "summarize_search",
            "汇总每文件数量、行范围和最多 12 个代表引用，不读正文；truncated=true 表示数量为下界。",
            json!({"search_id":string(),"offset":{"type":"integer","minimum":0}}),
            json!(["search_id"]),
        ),
        make(
            "read_log_context",
            "读取已验证引用附近的小窗口；前后默认 3 行、各最多 10 行/合计 16 KiB。",
            json!({"reference":reference(),"before":{"type":"integer","minimum":0,"maximum":10},"after":{"type":"integer","minimum":0,"maximum":10}}),
            json!(["reference"]),
        ),
        make(
            "read_log_segment",
            "读取长日志行片段。传 search_id 定位原匹配，或用 0-based Unicode start_character；默认 2048，最多 4096 字符。",
            json!({"reference":reference(),"search_id":string(),"start_character":{"type":"integer","minimum":0},"max_characters":{"type":"integer","minimum":1,"maximum":4096}}),
            json!(["reference"]),
        ),
        make(
            "show_search",
            "创建/激活 AI 搜索标签，使用 query 或预定义 filter；文件范围须传 document_id。",
            json!({"scope":choice(&["current","open","directory"]),"document_id":id(),"query":string(),"filter_id":string(),"case_sensitive":boolean(),"regex":boolean()}),
            json!(["scope"]),
        ),
        make(
            "control_search",
            "读取 AI 搜索标签状态/引用（每页 20，不含正文），或取消、清除该标签。",
            json!({"search_tab":string(),"action":choice(&["status","results","cancel","clear"]),"offset":{"type":"integer","minimum":0}}),
            json!(["search_tab", "action"]),
        ),
        make(
            "append_search",
            "向目标文件搜索框追加文字但不执行搜索；保留草稿和选项并返回最终 query。",
            json!({"document_id":id(),"version":string(),"text":string()}),
            json!(["document_id", "version", "text"]),
        ),
        make(
            "list_filters",
            "读取本地预定义过滤器的 ID、名称、表达式和正则选项，不访问云端。",
            json!({"offset":{"type":"integer","minimum":0}}),
            json!([]),
        ),
        make(
            "list_colors",
            "列出可用于关键词高亮的现有颜色标签；highlight_keyword 前先调用，不创建标签。",
            json!({"offset":{"type":"integer","minimum":0}}),
            json!([]),
        ),
        make(
            "set_marks",
            "设置/移除已验证行的书签。分析时只标记少量决定性事件；幂等且不修改源文件。",
            json!({"references":{"type":"array","items":reference(),"minItems":1,"maxItems":100},"marked":boolean()}),
            json!(["references", "marked"]),
        ),
        make(
            "highlight_keyword",
            "设置、更新或移除打开文件的精确关键词颜色规则；color_label_id 必须来自 list_colors。",
            json!({"document_id":id(),"version":string(),"keyword":string(),"color_label_id":string(),"case_sensitive":boolean(),"action":choice(&["set","remove"])}),
            json!(["document_id", "version", "keyword", "action"]),
        ),
        make(
            "text_mark",
            "新增、更新或移除文字注释；更新/移除须传 mark_id，text 单行且最多 128 字符。",
            json!({"reference":reference(),"action":choice(&["add","update","remove"]),"mark_id":string(),"text":{"type":"string","maxLength":128}}),
            json!(["reference", "action"]),
        ),
        make(
            "list_marks",
            "按源行分页取得文件中的行书签和文字注释。",
            json!({"document_id":id(),"version":string(),"start_line":id()}),
            json!(["document_id", "version"]),
        ),
        make(
            "navigate",
            "激活文件并跳到源行、首尾或搜索命中。line 需 reference；result 使用 search_id 和 1-based result_index。",
            json!({"action":choice(&["line","result","start","end","next","previous"]),"reference":reference(),"document_id":id(),"search_id":string(),"result_index":id()}),
            json!(["action"]),
        ),
        make(
            "list_skills",
            "列出本会话启用的技能。",
            json!({}),
            json!([]),
        ),
        make(
            "read_skill",
            "读取已启用技能的 SKILL.md 或 UTF-8 引用；用字符 offset 分页，不执行脚本。",
            json!({"skill_id":string(),"path":string(),"offset":{"type":"integer","minimum":0}}),
            json!(["skill_id", "path"]),
        ),
    ]
}
pub fn validate_call(call: &ToolCall) -> Result<()> {
    let definition = tool_definitions()
        .into_iter()
        .find(|d| d.name == call.name)
        .ok_or_else(|| anyhow::anyhow!("Unknown tool"))?;
    validate(&call.arguments, &definition.parameters)
}
fn validate(value: &Value, schema: &Value) -> Result<()> {
    match schema["type"].as_str() {
        Some("object") => {
            let object = value
                .as_object()
                .ok_or_else(|| anyhow::anyhow!("Expected object"))?;
            for required in schema["required"].as_array().into_iter().flatten() {
                if object.get(required.as_str().unwrap_or_default()).is_none() {
                    bail!("Missing argument: {required}");
                }
            }
            for (key, value) in object {
                let Some(property) = schema["properties"].get(key) else {
                    bail!("Unknown argument: {key}");
                };
                validate(value, property)?;
            }
        }
        Some("string") => {
            let text = value
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Expected string"))?;
            if schema["maxLength"]
                .as_u64()
                .is_some_and(|max| text.chars().count() as u64 > max)
            {
                bail!("String too long");
            }
            if schema["enum"]
                .as_array()
                .is_some_and(|values| !values.contains(value))
            {
                bail!("Invalid choice");
            }
        }
        Some("integer") => {
            let n = value
                .as_u64()
                .ok_or_else(|| anyhow::anyhow!("Expected non-negative integer"))?;
            if schema["minimum"].as_u64().is_some_and(|min| n < min)
                || schema["maximum"].as_u64().is_some_and(|max| n > max)
            {
                bail!("Number out of range");
            }
        }
        Some("boolean") if !value.is_boolean() => bail!("Expected boolean"),
        Some("array") => {
            let values = value
                .as_array()
                .ok_or_else(|| anyhow::anyhow!("Expected array"))?;
            if schema["minItems"]
                .as_u64()
                .is_some_and(|n| values.len() < n as usize)
                || schema["maxItems"]
                    .as_u64()
                    .is_some_and(|n| values.len() > n as usize)
            {
                bail!("Invalid list length");
            }
            for value in values {
                validate(value, &schema["items"])?;
            }
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_bounded_directory_and_ripgrep_interfaces() {
        let names = tool_definitions()
            .into_iter()
            .map(|tool| tool.name)
            .collect::<Vec<_>>();
        for name in [
            "list_log_directory",
            "list_source_workspaces",
            "rg_list_files",
            "rg_search",
            "rg_count",
        ] {
            assert!(names.contains(&name), "missing {name}");
        }

        let valid = ToolCall {
            id: "search".into(),
            name: "rg_search".into(),
            arguments: json!({
                "root": 0,
                "query": "timeout",
                "globs": ["*.rs", "!target/**"],
                "max_results": 200
            }),
        };
        assert!(validate_call(&valid).is_ok());

        let project_file = ToolCall {
            id: "open-project-file".into(),
            name: "open_file".into(),
            arguments: json!({"root":0,"path":"src/service.rs"}),
        };
        assert!(validate_call(&project_file).is_ok());

        let unbounded = ToolCall {
            arguments: json!({"root":0,"query":"timeout","max_results":201}),
            ..valid
        };
        assert!(validate_call(&unbounded).is_err());
    }

    #[test]
    fn defers_optional_groups_and_hides_unavailable_extensions() {
        let available = BTreeSet::from([ToolGroup::Evidence, ToolGroup::Files]);
        let initial = tool_definitions_for(&BTreeSet::new(), &available);
        let names = initial.iter().map(|tool| tool.name).collect::<Vec<_>>();
        assert_eq!(names.len(), 5);
        assert!(names.contains(&"load_tool_group"));
        assert!(!names.contains(&"read_logs"));
        assert_eq!(
            initial[0].parameters["properties"]["group"]["enum"],
            json!(["evidence", "files"])
        );

        let loaded = BTreeSet::from([ToolGroup::Evidence]);
        let names = tool_definitions_for(&loaded, &available)
            .into_iter()
            .map(|tool| tool.name)
            .collect::<Vec<_>>();
        assert!(names.contains(&"read_logs"));
        assert!(!names.contains(&"open_file"));
    }

    #[test]
    fn optional_groups_remain_small_and_focused() {
        for group in ToolGroup::OPTIONAL {
            let count = tool_definitions()
                .iter()
                .filter(|tool| group_for_tool(tool.name) == group)
                .count();
            assert!(count < 10, "{} contains {count} tools", group.id());
        }
    }
}
