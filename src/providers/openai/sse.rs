use std::collections::HashMap;

use crate::agent::message::ToolCallId;
use crate::providers::types::{CompletionReason, ProviderError, ProviderEvent, ToolCall, Usage};

#[derive(Debug, Default)]
pub struct ResponsesEventMapper {
    calls: HashMap<String, PendingToolCall>,
}

#[derive(Debug)]
struct PendingToolCall {
    id: ToolCallId,
    name: String,
    arguments: String,
}

impl ResponsesEventMapper {
    pub fn map(&mut self, frame: &SseFrame) -> Result<Vec<ProviderEvent>, ProviderError> {
        let value = frame
            .json_data()
            .map_err(|_| ProviderError::InvalidResponse)?;
        let kind = value
            .get("type")
            .and_then(serde_json::Value::as_str)
            .or(frame.event.as_deref())
            .ok_or(ProviderError::InvalidResponse)?;
        let mut events = Vec::new();
        match kind {
            "response.output_text.delta" => {
                let delta = value
                    .get("delta")
                    .and_then(serde_json::Value::as_str)
                    .ok_or(ProviderError::InvalidResponse)?;
                events.push(ProviderEvent::TextDelta(delta.to_owned()));
            }
            "response.output_item.added" => {
                let item = value.get("item").ok_or(ProviderError::InvalidResponse)?;
                if item.get("type").and_then(serde_json::Value::as_str) == Some("function_call") {
                    let item_id = string_field(item, "id")?;
                    let call_id = string_field(item, "call_id")?;
                    let name = string_field(item, "name")?;
                    let arguments = item
                        .get("arguments")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("")
                        .to_owned();
                    let id = ToolCallId::new(call_id);
                    self.calls.insert(
                        item_id.to_owned(),
                        PendingToolCall {
                            id: id.clone(),
                            name: name.to_owned(),
                            arguments,
                        },
                    );
                    events.push(ProviderEvent::ToolCallStarted {
                        id,
                        name: name.to_owned(),
                    });
                }
            }
            "response.function_call_arguments.delta" => {
                let item_id = string_field(&value, "item_id")?;
                let delta = string_field(&value, "delta")?;
                if let Some(call) = self.calls.get_mut(item_id) {
                    call.arguments.push_str(delta);
                    events.push(ProviderEvent::ToolCallArgumentsDelta {
                        id: call.id.clone(),
                        delta: delta.to_owned(),
                    });
                }
            }
            "response.function_call_arguments.done" => {
                let item_id = string_field(&value, "item_id")?;
                let arguments = string_field(&value, "arguments")?;
                if let Some(call) = self.calls.remove(item_id) {
                    events.push(ProviderEvent::ToolCallCompleted(ToolCall {
                        id: call.id,
                        name: call.name,
                        arguments: arguments.to_owned(),
                    }));
                }
            }
            "response.completed" => {
                let response = value
                    .get("response")
                    .ok_or(ProviderError::InvalidResponse)?;
                if let Some(usage) = response.get("usage") {
                    let input_tokens = usage
                        .get("input_tokens")
                        .and_then(serde_json::Value::as_u64)
                        .ok_or(ProviderError::InvalidResponse)?;
                    let output_tokens = usage
                        .get("output_tokens")
                        .and_then(serde_json::Value::as_u64)
                        .ok_or(ProviderError::InvalidResponse)?;
                    events.push(ProviderEvent::Usage(Usage {
                        input_tokens,
                        output_tokens,
                    }));
                }
                let has_calls = response
                    .get("output")
                    .and_then(serde_json::Value::as_array)
                    .is_some_and(|items| {
                        items.iter().any(|item| {
                            item.get("type").and_then(serde_json::Value::as_str)
                                == Some("function_call")
                        })
                    });
                events.push(ProviderEvent::Completed(if has_calls {
                    CompletionReason::ToolCalls
                } else {
                    CompletionReason::EndTurn
                }));
            }
            "response.incomplete" => {
                let reason = value
                    .get("response")
                    .and_then(|response| response.get("incomplete_details"))
                    .and_then(|details| details.get("reason"))
                    .and_then(serde_json::Value::as_str);
                events.push(ProviderEvent::Completed(match reason {
                    Some("content_filter") => CompletionReason::ContentFilter,
                    _ => CompletionReason::LengthLimit,
                }));
            }
            "response.failed" | "error" => {
                let error = value
                    .get("response")
                    .and_then(|r| r.get("error"))
                    .or_else(|| value.get("error"))
                    .unwrap_or(&value);
                return Err(ProviderError::Api {
                    status: 0,
                    code: error
                        .get("code")
                        .and_then(serde_json::Value::as_str)
                        .and_then(safe_code),
                });
            }
            // Lifecycle, metadata, content-part, and future typed events do not alter shared state.
            _ => {}
        }
        Ok(events)
    }

    /// Discards any calls whose argument stream ended before its `done` event.
    pub fn finish(&mut self) {
        self.calls.clear();
    }
}

fn string_field<'a>(value: &'a serde_json::Value, field: &str) -> Result<&'a str, ProviderError> {
    value
        .get(field)
        .and_then(serde_json::Value::as_str)
        .ok_or(ProviderError::InvalidResponse)
}

fn safe_code(code: &str) -> Option<String> {
    let safe: String = code
        .chars()
        .take(64)
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '_' || *ch == '-')
        .collect();
    (!safe.is_empty()).then_some(safe)
}

const DEFAULT_MAX_FRAME_SIZE: usize = 1024 * 1024;

#[derive(Debug, thiserror::Error, Clone, Copy, PartialEq, Eq)]
pub enum SseError {
    #[error("server-sent event frame exceeds its size limit")]
    FrameTooLarge,
    #[error("server-sent event contains invalid UTF-8")]
    InvalidUtf8,
    #[error("server-sent event data is not valid JSON")]
    MalformedJson,
    #[error("stream ended before the current event frame was complete")]
    IncompleteFrame,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseFrame {
    pub event: Option<String>,
    pub data: String,
}

impl SseFrame {
    pub fn json_data(&self) -> Result<serde_json::Value, SseError> {
        serde_json::from_str(&self.data).map_err(|_| SseError::MalformedJson)
    }
}

#[derive(Debug)]
pub struct SseDecoder {
    max_frame_size: usize,
    line_buffer: Vec<u8>,
    frame_size: usize,
    event: Option<String>,
    data_lines: Vec<String>,
    has_data: bool,
}

impl Default for SseDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl SseDecoder {
    pub fn new() -> Self {
        Self::with_max_frame_size(DEFAULT_MAX_FRAME_SIZE)
    }

    pub fn with_max_frame_size(max_frame_size: usize) -> Self {
        Self {
            max_frame_size: max_frame_size.max(1),
            line_buffer: Vec::new(),
            frame_size: 0,
            event: None,
            data_lines: Vec::new(),
            has_data: false,
        }
    }

    pub fn push(&mut self, chunk: &[u8]) -> Result<Vec<SseFrame>, SseError> {
        let mut frames = Vec::new();
        for byte in chunk {
            self.line_buffer.push(*byte);
            if self.frame_size.saturating_add(self.line_buffer.len()) > self.max_frame_size {
                return Err(SseError::FrameTooLarge);
            }
            if *byte == b'\n' {
                let mut line = std::mem::take(&mut self.line_buffer);
                self.frame_size = self.frame_size.saturating_add(line.len());
                line.pop();
                if let Some(frame) = self.process_line(&line)? {
                    frames.push(frame);
                }
            }
        }
        Ok(frames)
    }

    pub fn finish(&mut self) -> Result<(), SseError> {
        if !self.line_buffer.is_empty() {
            let line = std::mem::take(&mut self.line_buffer);
            self.frame_size = self.frame_size.saturating_add(line.len());
            self.process_line(&line)?;
        }
        if self.event.is_some() || self.has_data {
            return Err(SseError::IncompleteFrame);
        }
        Ok(())
    }

    fn process_line(&mut self, line: &[u8]) -> Result<Option<SseFrame>, SseError> {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        let line = std::str::from_utf8(line).map_err(|_| SseError::InvalidUtf8)?;
        if line.is_empty() {
            if self.has_data {
                let frame = SseFrame {
                    event: self.event.take(),
                    data: self.data_lines.join("\n"),
                };
                self.data_lines.clear();
                self.has_data = false;
                self.frame_size = 0;
                return Ok(Some(frame));
            }
            self.event = None;
            self.frame_size = 0;
            return Ok(None);
        }
        if line.starts_with(':') {
            return Ok(None);
        }

        let (field, value) = line.split_once(':').unwrap_or((line, ""));
        let value = value.strip_prefix(' ').unwrap_or(value);
        match field {
            "event" => self.event = (!value.is_empty()).then(|| value.to_owned()),
            "data" => {
                self.has_data = true;
                self.data_lines.push(value.to_owned());
            }
            _ => {}
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::{SseDecoder, SseError};

    #[test]
    fn parses_frames_across_every_byte_split() {
        let input = "event: response.output_text.delta\r\ndata: {\"text\":\"hello 🌍\"}\r\n\r\n";
        let bytes = input.as_bytes();

        for split in 0..=bytes.len() {
            let mut decoder = SseDecoder::new();
            let mut frames = decoder.push(&bytes[..split]).unwrap();
            frames.extend(decoder.push(&bytes[split..]).unwrap());
            decoder.finish().unwrap();

            assert_eq!(frames.len(), 1, "split at byte {split}");
            assert_eq!(
                frames[0].event.as_deref(),
                Some("response.output_text.delta")
            );
            assert_eq!(frames[0].data, "{\"text\":\"hello 🌍\"}");
        }
    }

    #[test]
    fn handles_comments_multiple_frames_and_blank_data() {
        let mut decoder = SseDecoder::new();
        let frames = decoder
            .push(
                b": heartbeat\r\nevent: unknown.event\r\ndata:\r\n\r\ndata: {\"ok\":\ndata: true}\n\n",
            )
            .unwrap();
        decoder.finish().unwrap();

        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].event.as_deref(), Some("unknown.event"));
        assert_eq!(frames[0].data, "");
        assert_eq!(frames[1].data, "{\"ok\":\ntrue}");
    }

    #[test]
    fn reports_malformed_json_separately_from_sse_framing() {
        let mut decoder = SseDecoder::new();
        let frames = decoder.push(b"data: {not-json}\n\n").unwrap();

        assert!(matches!(
            frames[0].json_data(),
            Err(SseError::MalformedJson)
        ));
    }

    #[test]
    fn reports_eof_with_an_incomplete_frame() {
        let mut decoder = SseDecoder::new();
        assert!(
            decoder
                .push(b"event: response.completed\ndata: {\"ok\":true}")
                .unwrap()
                .is_empty()
        );

        assert!(matches!(decoder.finish(), Err(SseError::IncompleteFrame)));
    }

    #[test]
    fn bounds_pending_frame_bytes() {
        let mut decoder = SseDecoder::with_max_frame_size(8);

        assert!(matches!(
            decoder.push(b"data: 1234"),
            Err(SseError::FrameTooLarge)
        ));
    }
}

#[cfg(test)]
mod mapper_tests {
    use super::{ResponsesEventMapper, SseFrame};
    use crate::agent::message::ToolCallId;
    use crate::providers::types::{
        CompletionReason, ProviderError, ProviderEvent, ToolCall, Usage,
    };

    fn frame(event: &str, data: &str) -> SseFrame {
        SseFrame {
            event: Some(event.into()),
            data: data.into(),
        }
    }

    #[test]
    fn maps_text_delta_and_ignores_unknown_event() {
        let mut mapper = ResponsesEventMapper::default();
        assert_eq!(
            mapper
                .map(&frame(
                    "response.output_text.delta",
                    r#"{"type":"response.output_text.delta","delta":"hi"}"#
                ))
                .unwrap(),
            vec![ProviderEvent::TextDelta("hi".into())]
        );
        assert!(
            mapper
                .map(&frame("response.created", r#"{"type":"response.created"}"#))
                .unwrap()
                .is_empty()
        );
        assert!(
            mapper
                .map(&frame("vendor.future", r#"{"type":"vendor.future"}"#))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn accumulates_tool_arguments_and_completes_only_on_done() {
        let mut mapper = ResponsesEventMapper::default();
        let added = mapper.map(&frame("response.output_item.added", r#"{"type":"response.output_item.added","output_index":0,"item":{"type":"function_call","id":"fc_1","call_id":"call_1","name":"read_file","arguments":""}}"#)).unwrap();
        assert_eq!(
            added,
            vec![ProviderEvent::ToolCallStarted {
                id: ToolCallId::new("call_1"),
                name: "read_file".into()
            }]
        );
        assert_eq!(mapper.map(&frame("response.function_call_arguments.delta", r#"{"type":"response.function_call_arguments.delta","item_id":"fc_1","delta":"{\"path\":"}"#)).unwrap(), vec![ProviderEvent::ToolCallArgumentsDelta { id: ToolCallId::new("call_1"), delta: "{\"path\":".into() }]);
        assert!(mapper.map(&frame("response.function_call_arguments.delta", r#"{"type":"response.function_call_arguments.delta","item_id":"fc_1","delta":"\"a\"}"}"#)).unwrap().len() == 1);
        assert_eq!(mapper.map(&frame("response.function_call_arguments.done", r#"{"type":"response.function_call_arguments.done","item_id":"fc_1","arguments":"{\"path\":\"a\"}"}"#)).unwrap(), vec![ProviderEvent::ToolCallCompleted(ToolCall { id: ToolCallId::new("call_1"), name: "read_file".into(), arguments: r#"{"path":"a"}"#.into() })]);
    }

    #[test]
    fn incomplete_tool_arguments_never_complete_at_eof() {
        let mut mapper = ResponsesEventMapper::default();
        mapper.map(&frame("response.output_item.added", r#"{"type":"response.output_item.added","item":{"type":"function_call","id":"fc_1","call_id":"call_1","name":"read_file","arguments":""}}"#)).unwrap();
        mapper
            .map(&frame(
                "response.function_call_arguments.delta",
                r#"{"type":"response.function_call_arguments.delta","item_id":"fc_1","delta":"{"}"#,
            ))
            .unwrap();
        mapper.finish();
    }

    #[test]
    fn maps_response_completion_usage_and_failures() {
        let mut mapper = ResponsesEventMapper::default();
        assert_eq!(mapper.map(&frame("response.completed", r#"{"type":"response.completed","response":{"status":"completed","output":[],"usage":{"input_tokens":3,"output_tokens":5}}}"#)).unwrap(), vec![ProviderEvent::Usage(Usage { input_tokens: 3, output_tokens: 5 }), ProviderEvent::Completed(CompletionReason::EndTurn)]);
        assert_eq!(
            mapper
                .map(&frame(
                    "response.incomplete",
                    r#"{"type":"response.incomplete","response":{"incomplete_details":{"reason":"content_filter"}}}"#
                ))
                .unwrap(),
            vec![ProviderEvent::Completed(CompletionReason::ContentFilter)]
        );
        assert!(
            matches!(mapper.map(&frame("error", r#"{"type":"error","code":"bad_request","message":"secret"}"#)), Err(ProviderError::Api { status: 0, code: Some(code) }) if code == "bad_request")
        );
        assert!(
            matches!(mapper.map(&frame("response.failed", r#"{"type":"response.failed","response":{"error":{"code":"failed","message":"secret"}}}"#)), Err(ProviderError::Api { status: 0, code: Some(code) }) if code == "failed")
        );
    }
}
