//! Tool names the way Claude Code spells them.
//!
//! Claude Code's own tools have fixed names, and a request that names its tools differently looks
//! like someone else's. A tool whose name is one of these in any casing goes out with the casing
//! Claude Code uses; the model's calls are mapped back to the names velox declared.

use velox_sdk::ToolDeclaration;

const CLAUDE_CODE_TOOLS: [&str; 17] = [
    "Read",
    "Write",
    "Edit",
    "Bash",
    "Grep",
    "Glob",
    "AskUserQuestion",
    "EnterPlanMode",
    "ExitPlanMode",
    "KillShell",
    "NotebookEdit",
    "Skill",
    "Task",
    "TaskOutput",
    "TodoWrite",
    "WebFetch",
    "WebSearch",
];

/// The name the API is given for the tool velox calls `name`.
pub(crate) fn to_wire(name: &str) -> String {
    CLAUDE_CODE_TOOLS
        .iter()
        .find(|canonical| canonical.eq_ignore_ascii_case(name))
        .map_or(name, |canonical| canonical)
        .to_owned()
}

/// The name of the declared tool that a call the API names `wire` is for.
pub(crate) fn from_wire(wire: &str, declared: &[ToolDeclaration]) -> String {
    declared
        .iter()
        .find(|tool| tool.name.eq_ignore_ascii_case(wire))
        .map_or(wire, |tool| &tool.name)
        .to_owned()
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use velox_sdk::ExecutionMode;

    use super::*;

    fn declared(name: &str) -> ToolDeclaration {
        ToolDeclaration {
            name: name.to_owned(),
            description: String::new(),
            parameters: json!({}),
            mode: ExecutionMode::Parallel,
        }
    }

    #[test]
    fn a_claude_code_tool_goes_out_in_its_casing() {
        assert_eq!(to_wire("read"), "Read");
        assert_eq!(to_wire("BASH"), "Bash");
        assert_eq!(to_wire("Grep"), "Grep");
    }

    #[test]
    fn any_other_tool_keeps_its_name() {
        assert_eq!(to_wire("todo_write"), "todo_write");
    }

    #[test]
    fn a_call_goes_back_to_the_name_that_was_declared() {
        let tools = [declared("read"), declared("lint")];

        assert_eq!(from_wire("Read", &tools), "read");
        assert_eq!(from_wire("lint", &tools), "lint");
    }

    #[test]
    fn a_call_to_a_tool_that_was_not_declared_keeps_the_name_it_came_with() {
        assert_eq!(from_wire("Write", &[declared("read")]), "Write");
    }
}
