//! Forwards session output to a WebView channel.

use std::sync::Arc;

use rove_session::{OutputChunk, OutputSink};
use rove_ssh::Progress;
use tauri::ipc::{Channel, InvokeResponseBody};

/// Reports connection steps on a session's output channel as JSON messages
/// `{ "progress": ConnectStep }`, ahead of any terminal output.
pub fn progress_to(channel: &Channel<InvokeResponseBody>) -> Progress {
    let channel = channel.clone();
    Arc::new(move |step| {
        let json = serde_json::json!({ "progress": step });
        if channel
            .send(InvokeResponseBody::Json(json.to_string()))
            .is_err()
        {
            tracing::debug!("progress channel closed");
        }
    })
}

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
