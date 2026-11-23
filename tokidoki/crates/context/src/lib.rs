//! Everything that gets pushed into inference: system prompt, registered tools, message history.
//! Vendor-neutral: each provider format translates this to its own wire shape.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Context {
    pub system: Option<String>,
    pub tools: Vec<Tool>,
    pub messages: Vec<Message>,
}

impl Context {
    pub fn new(system: impl Into<String>) -> Self {
        Self {
            system: Some(system.into()),
            ..Default::default()
        }
    }

    pub fn register(&mut self, tool: Tool) {
        self.tools.push(tool);
    }

    pub fn push(&mut self, message: Message) {
        self.messages.push(message);
    }

    /// Calls of tools that aren't offered (any more), and their results, as plain text.
    /// A history that calls a tool the request doesn't declare is refused by some providers
    /// and answered with nothing by others; said in words, it is just what happened.
    pub fn retell_unoffered_calls(&mut self) {
        let mut retold: HashMap<String, String> = HashMap::new();
        for message in &mut self.messages {
            for block in &mut message.content {
                let text = match block {
                    Block::ToolCall { id, name, input }
                        if !self.tools.iter().any(|t| t.name == *name) =>
                    {
                        retold.insert(id.clone(), name.clone());
                        format!("(I used the {name} tool, which I don't have now, with {input}.)")
                    }
                    Block::ToolResult {
                        call_id,
                        content,
                        is_error,
                    } => match retold.get(call_id) {
                        Some(name) if *is_error => format!("(The {name} tool failed: {content})"),
                        Some(name) => format!("(The {name} tool returned:)\n{content}"),
                        None => continue,
                    },
                    _ => continue,
                };
                *block = Block::Text { text };
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: Vec<Block>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Block {
    Text {
        text: String,
    },
    /// The model asking to run a tool. `input` is the parsed JSON arguments.
    ToolCall {
        id: String,
        name: String,
        input: Value,
    },
    /// What a tool returned, answering the `ToolCall` with the same id.
    ToolResult {
        call_id: String,
        content: String,
        is_error: bool,
    },
    /// The model's reasoning. `signature` is set by Anthropic, which needs it to accept
    /// the block back; unsigned thinking (OpenAI-style vendors) is never sent back.
    Thinking {
        text: String,
        signature: Option<String>,
    },
    /// Anthropic's encrypted reasoning. Only Anthropic accepts it back.
    RedactedThinking {
        data: String,
    },
}

impl Message {
    pub fn user(text: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: vec![Block::Text { text: text.into() }],
        }
    }

    pub fn assistant(text: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: vec![Block::Text { text: text.into() }],
        }
    }

    pub fn tool_result(call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: vec![Block::ToolResult {
                call_id: call_id.into(),
                content: content.into(),
                is_error: false,
            }],
        }
    }

    /// All text blocks joined.
    pub fn text(&self) -> String {
        self.content
            .iter()
            .filter_map(|b| match b {
                Block::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    /// The `ToolCall` blocks, as (id, name, input).
    pub fn tool_calls(&self) -> impl Iterator<Item = (&str, &str, &Value)> {
        self.content.iter().filter_map(|b| match b {
            Block::ToolCall { id, name, input } => Some((id.as_str(), name.as_str(), input)),
            _ => None,
        })
    }
}

/// A tool the model may call. `input_schema` is a JSON Schema object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn builds_up_context() {
        let mut ctx = Context::new("Be terse.");
        ctx.register(Tool {
            name: "weather".into(),
            description: "Current weather for a city".into(),
            input_schema: json!({ "type": "object", "properties": { "city": { "type": "string" } } }),
        });
        ctx.push(Message::user("Weather in Paris?"));
        ctx.push(Message {
            role: Role::Assistant,
            content: vec![
                Block::Text {
                    text: "Checking.".into(),
                },
                Block::ToolCall {
                    id: "c1".into(),
                    name: "weather".into(),
                    input: json!({ "city": "Paris" }),
                },
            ],
        });
        ctx.push(Message::tool_result("c1", "18C, sunny"));

        assert_eq!(ctx.messages[1].text(), "Checking.");
        let calls: Vec<_> = ctx.messages[1].tool_calls().collect();
        assert_eq!(calls, [("c1", "weather", &json!({ "city": "Paris" }))]);
    }

    #[test]
    fn calls_of_a_tool_no_longer_offered_are_retold_as_text() {
        let tool = |name: &str| Tool {
            name: name.into(),
            description: String::new(),
            input_schema: json!({ "type": "object" }),
        };
        let call = |id: &str, name: &str| Message {
            role: Role::Assistant,
            content: vec![Block::ToolCall {
                id: id.into(),
                name: name.into(),
                input: json!({ "q": "x" }),
            }],
        };
        let mut ctx = Context::default();
        ctx.register(tool("weather"));
        ctx.push(call("c1", "search"));
        ctx.push(Message::tool_result("c1", "three pages"));
        ctx.push(call("c2", "weather"));
        ctx.push(Message::tool_result("c2", "18C"));
        let kept = ctx.messages[2..].to_vec();

        ctx.retell_unoffered_calls();

        assert_eq!(
            ctx.messages[0].text(),
            r#"(I used the search tool, which I don't have now, with {"q":"x"}.)"#
        );
        assert_eq!(ctx.messages[0].tool_calls().count(), 0);
        assert_eq!(
            ctx.messages[1].text(),
            "(The search tool returned:)\nthree pages"
        );
        assert_eq!(ctx.messages[1].role, Role::User);
        assert_eq!(ctx.messages[2..], kept);
    }
}
