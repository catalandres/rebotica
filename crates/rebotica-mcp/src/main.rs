//! Thin wrapper around [`rebotica_mcp::serve_stdio`] so the MCP server can
//! be invoked as a standalone binary in addition to via `rbtc mcp serve`.

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    rebotica_mcp::serve_stdio().await
}
