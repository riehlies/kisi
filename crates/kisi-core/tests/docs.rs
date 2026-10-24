//! The user documentation in `docs/` against the program, so that it cannot drift:
//! `docs/cli.md` carries `kisi help` verbatim and names every command and option,
//! `docs/preparing-a-board.md` names every gap of `kisi gaps`.
//!
//! `KISI_BLESS_DOCS=1 cargo test -p kisi-core --test docs` writes the current help text into
//! `docs/cli.md` (the descriptions of new commands, options and gaps are written by hand).

use kisi_core::commands::{COMMANDS, HELP};

fn doc_path(name: &str) -> String {
    format!("{}/../../docs/{name}", env!("CARGO_MANIFEST_DIR"))
}

fn read(name: &str) -> String {
    std::fs::read_to_string(doc_path(name)).unwrap_or_else(|e| panic!("docs/{name}: {e}"))
}

/// Options that `commands::parse_args` accepts: the match arms `"--name" =>` in its source.
fn parsed_options() -> Vec<&'static str> {
    let src = include_str!("../src/commands.rs");
    let v: Vec<&str> = src
        .lines()
        .map(str::trim_start)
        .filter(|l| l.starts_with("\"--"))
        .filter_map(|l| l[1..].split_once("\" =>").map(|(o, _)| o))
        .collect();
    assert!(v.len() > 10, "options not found in commands.rs ({v:?})");
    v
}

#[test]
fn docs_cli_help() {
    const BEGIN: &str = "<!-- BEGIN kisi help";
    const END: &str = "<!-- END kisi help -->";
    let doc = read("cli.md");
    let b = doc.find(BEGIN).expect("docs/cli.md: BEGIN marker");
    let b = b + doc[b..].find('\n').expect("line end") + 1;
    let e = doc.find(END).expect("docs/cli.md: END marker");
    let want = format!("```text\n{HELP}\n```\n");
    if doc[b..e] != want {
        if std::env::var_os("KISI_BLESS_DOCS").is_some() {
            std::fs::write(doc_path("cli.md"), format!("{}{want}{}", &doc[..b], &doc[e..])).expect("write docs/cli.md");
        } else {
            panic!("docs/cli.md: the `kisi help` block differs from kisi_core::commands::HELP – run `KISI_BLESS_DOCS=1 cargo test -p kisi-core --test docs`");
        }
    }
    for c in COMMANDS {
        assert!(doc.contains(&format!("### `{c}`")), "docs/cli.md: no section \"### `{c}`\"");
    }
    for o in parsed_options() {
        assert!(HELP.contains(o), "kisi help does not mention {o}");
        assert!(doc.contains(&format!("| `{o}")), "docs/cli.md: option {o} missing in the options table");
    }
}

#[test]
fn docs_gap_ids() {
    // the gap ids: the first string after every `gap(` call in gaps.rs
    let src = include_str!("../src/gaps.rs");
    let mut ids = vec![];
    for (i, _) in src.match_indices("gap(") {
        if src[..i].ends_with("fn ") || src[..i].ends_with('_') {
            continue;
        }
        let rest = &src[i + 4..];
        let t = rest.trim_start();
        if let Some(t) = t.strip_prefix('"') {
            ids.push(&t[..t.find('"').unwrap()]);
        }
    }
    assert!(ids.len() >= 10, "gap ids not found in gaps.rs ({ids:?})");
    let doc = read("preparing-a-board.md");
    for id in ids {
        assert!(doc.contains(&format!("`{id}`")), "docs/preparing-a-board.md: gap `{id}` is not described");
    }
}
