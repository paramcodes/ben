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
