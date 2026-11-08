//! Forwards session output to a WebView channel.

use rove_session::{OutputChunk, OutputSink};
use tauri::ipc::{Channel, InvokeResponseBody};

/// Sends terminal bytes as raw `ArrayBuffer` messages and the exit as a JSON message
/// `{ "exit": { "code": number | null, "signal": string | null } }`.
pub struct ChannelSink(pub Channel<InvokeResponseBody>);

impl OutputSink for ChannelSink {
    fn send(&self, chunk: OutputChunk) -> bool {
        let body = match chunk {
            OutputChunk::Data(data) => InvokeResponseBody::Raw(data.to_vec()),
            OutputChunk::Exit(status) => {
                let json = serde_json::json!({ "exit": status });
                InvokeResponseBody::Json(json.to_string())
            }
        };
        self.0.send(body).is_ok()
    }
}
