# kisi – Claude-Code-Plugin (MCP-Server + Skill)

1. MCP-Server bauen und in den PATH legen: `cargo install --path crates/kisi-mcp`
   (für `kisi_open_viewer` zusätzlich `dist/kisi.html` neben das Programm legen oder `KISI_VIEWER` setzen)
2. In Claude Code: `/plugin marketplace add <Pfad oder Repo von kisi>` und `/plugin install kisi@kisi`

Andere MCP-Clients: `kisi-mcp` als stdio-Server eintragen, z. B.
`{"mcpServers": {"kisi": {"command": "kisi-mcp"}}}`.

Werkzeuge: `kisi_overview`, `kisi_net`, `kisi_channel_check`, `kisi_profiles`, `kisi_power`,
`kisi_report` (schreibt `analysis/kisi.json`), `kisi_open_viewer`.
Umgebung: `KISI_VIEWER` (Pfad zu kisi.html), `KISI_PROFILES` (eigene interfaces.json),
`KISI_NO_BROWSER` (Viewer-Datei nur schreiben).
