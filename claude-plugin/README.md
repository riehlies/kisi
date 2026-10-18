# kisi – Claude Code plugin (MCP server + skill)

1. Build the MCP server and put it on the PATH: `cargo install --path crates/kisi-mcp`
   (for `kisi_open_viewer`, also place `dist/kisi.html` next to the binary or set `KISI_VIEWER`)
2. In Claude Code: `/plugin marketplace add <path or repo of kisi>` and `/plugin install kisi@kisi`

Other MCP clients: register `kisi-mcp` as a stdio server, e.g.
`{"mcpServers": {"kisi": {"command": "kisi-mcp"}}}`.

Tools: `kisi_overview`, `kisi_interfaces`, `kisi_net`, `kisi_channel_check`, `kisi_profiles`, `kisi_power`,
`kisi_report` (writes `analysis/kisi.json`), `kisi_open_viewer`.
Environment: `KISI_VIEWER` (path to kisi.html), `KISI_PROFILES` (own interfaces.json),
`KISI_NO_BROWSER` (only write the viewer file).
