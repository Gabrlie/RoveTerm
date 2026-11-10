// Hides the console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // An agent starts the app's built-in MCP server through this bridge (see rove_ai::mcp).
    if std::env::args().nth(1).as_deref() == Some("--mcp-bridge") {
        std::process::exit(roveterm_lib::mcp_bridge());
    }
    roveterm_lib::run();
}
