# mntk — Media Naming Toolkit

A suite of fast, efficient command-line tools for naming local media files
using metadata from public sources.

`mntk` replaces a collection of bash + `curl` + `jq` + `dialog` scripts with
a single, well-tested Rust binary. The first tool, `mntk movie`, ports
`movie_search_and_rename_single` from `legacy/media_funcs.sh`.

## Status

`mntk movie` is functional. More tools (series/episodes) are planned.

## Requirements

- Rust (edition 2024 toolchain)
- An [OMDB API key](https://www.omdbapi.com/apikey.aspx) — free, and required
  by OMDB even for the basic API

## Building

```sh
make            # debug build
make release    # release build -> target/release/mntk
make install    # installs to $PREFIX/bin (default: ~/.local/bin)
make uninstall  # removes it again
```

`PREFIX` can be overridden: `make install PREFIX=/usr/local`
(sudo may be required for system prefixes). `make` also provides
`test`, `clippy`, `fmt`, `fmt-check`, `check`, and `clean` targets.
Plain `cargo build --release` works as well.

## `mntk movie`

Searches OMDB for the movie a file belongs to, then moves it into a
`<Title (Year)>/` folder under the **current working directory**:

```
The.Matrix.1999.1080p.mkv
    → The Matrix (1999)/The Matrix (1999) [imdbid-tt0133093].mkv
```

The `[imdbid-ttXXXXXXX]` tag is recognized by Jellyfin (and other media
servers) for library matching; the folder name stays clean.

### Usage

```
mntk movie [OPTIONS] <FILE>
```

| Flag | Description |
|---|---|
| `--search <TITLE>` | Search term. In a terminal this prefills the prompt (editable); outside a terminal it is required. |
| `--select <N>` | Pick result N (1-based) without prompting. |
| `--dry-run` | Print the planned move without touching anything. |
| `--force` | Overwrite an existing file at the destination. |
| `--no-imdb-id` | Do not embed the `[imdbid-…]` tag in the file name. |
| `--source <SOURCE>` | Metadata source (`omdb`). Global; may appear before or after the subcommand. |
| `--api-key <KEY>` | OMDB API key. Global; highest-priority key source. |
| `--config <PATH>` | Config file to use instead of the default location. Global. |

### Interactive (terminal)

```sh
cd ~/Downloads
mntk movie "The.Matrix.1999.1080p.mkv"
```

1. A prompt appears, prefilled with a term derived from the file name
   (bracketed release tags are dropped, and tokens stop after the first
   year-like one, so `The.Matrix.1999.1080p.mkv` becomes `The Matrix 1999`).
   Press Enter to accept, or type a new term.
2. Every result is shown in a menu as `Title (Year) - ttID`, plus a
   **Search again…** item. If the first search was wrong, pick it and the
   prompt reopens prefilled with your last term — no restart needed.
3. The file is moved. Ctrl+C at any point aborts with exit code 130.

### Non-interactive (no TTY)

Prompts are impossible without a terminal, so the command is fully
deterministic:

```sh
mntk movie movie.mkv --search "The Matrix" --select 1
```

- A single match is selected automatically; `--select <N>` picks explicitly.
- Several matches and no `--select` is an error that lists every candidate
  (with ids) so you can rerun with `--select`.
- The loop/retry behavior of interactive mode never happens; zero results
  are a plain error.

### Exit codes

| Code | Meaning |
|---|---|
| `0` | Success |
| `1` | Failure (missing file, no API key, no results, destination collision, network/HTTP error, …) |
| `2` | Command-line usage error (clap) |
| `130` | Interrupted (Ctrl+C) |

### Naming rules

- The invalid file-name character set `: \ / * " < > | ?` is replaced with
  `_`, leading whitespace is trimmed, and trailing dots/whitespace are
  stripped (the most restrictive common denominator, Windows included).
- The source file's extension is preserved (the legacy script hard-coded
  `.mkv`).
- The IMDB id is embedded only when well-formed (`tt` + 7–10 digits) and is
  normalized to lowercase.

## Configuration

Optionally place a YAML file at `~/.config/mntk/config.yaml`
(`$XDG_CONFIG_HOME/mntk/config.yaml`):

```yaml
source: omdb            # default metadata source
embed_imdb_id: true     # embed [imdbid-…] tags in file names
omdb:
  api_key: "your-key-here"
  base_url: "http://www.omdbapi.com"   # e.g. https://api.omdbbox.com for the paid API
```

A missing file is fine — everything has a default.

### API key precedence

1. `--api-key <KEY>`
2. `OMDB_API_KEY` environment variable
3. `OMDB_ID` environment variable (legacy name, kept for the old scripts)
4. `omdb.api_key` in the config file

## Workspace layout

| Crate | Package | Description |
|---|---|---|
| `crates/core` | `mntk-core` | Source-agnostic core: metadata source trait, naming rules, configuration |
| `crates/source-omdb` | `mntk-source-omdb` | OMDB metadata source implementation |
| `crates/cli` | `mntk` | The command-line interface (binary `mntk`) |

Providers implement `mntk_core::MovieSource`; the CLI depends only on that
trait. Adding TVDB/TMDB later is a new `crates/source-*` crate plus one more
`Source` variant — no changes to the core or the command flow.

## Development

```sh
cargo test --workspace          # 58 unit tests, no network required
cargo clippy --workspace --all-targets
cargo fmt
```

Tests never touch the network: the OMDB provider is tested against an
in-memory transport, and the CLI's decision logic against an in-memory
source.

## Reference

The original bash implementation this project replaces lives in `legacy/`
(`media_funcs.sh`, `rename_seq.sh`). Notable bugs fixed along the way: an
inverted `Response` check in the rename step, a hard-coded `.mkv`
extension, and unquoted curl arguments.
