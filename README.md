# mntk — Media Naming Toolkit

A suite of fast, efficient command-line tools for naming local media files
using metadata from public sources.

## Status

Under construction.

## Workspace layout

| Crate | Package | Description |
|---|---|---|
| `crates/core` | `mntk-core` | Source-agnostic core: metadata source abstraction, naming rules, configuration |
| `crates/source-omdb` | `mntk-source-omdb` | OMDB metadata source implementation |
| `crates/cli` | `mntk` | The command-line interface (binary `mntk`) |

## Building

```sh
cargo build --release
```

## Reference

The original bash implementation this project replaces lives in `legacy/`
(`media_funcs.sh`, `rename_seq.sh`).
