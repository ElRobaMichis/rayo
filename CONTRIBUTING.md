# Contributing to Rayo

Thanks for taking the time to help! Rayo is a small project and every bug report, idea, translation and pull
request makes a difference. Contributions are welcome in **English or Spanish**.

## Ways to help

- **Report a bug** — open an [issue](https://github.com/ElRobaMichis/rayo/issues/new/choose) with the steps to
  reproduce it, what you expected and what happened. Screenshots help a lot. If Rayo crashed, attach the end of
  `%LOCALAPPDATA%\Rayo\errores.log`.
- **Suggest a feature** — describe the problem you want solved before the solution; it makes it easier to find
  the right design.
- **Translate Rayo** — no programming needed. See the [translation guide](idiomas/LÉEME.md).
- **Send a pull request** — for anything bigger than a small fix, please open an issue first so we can agree on
  the approach before you spend time on it.

## Development setup

You need Windows 10/11 and [Rust](https://rustup.rs/) (stable, 1.85 or newer).

```bash
git clone https://github.com/ElRobaMichis/rayo.git
cd rayo
cargo build --release
cargo test --lib
```

Run your build with its own settings and index, so it never touches the Rayo you use day to day:

```bash
set RAYO_DATA_DIR=%TEMP%\rayo-dev
target\release\rayo.exe
```

A copy that is not the installed one never changes Windows registrations (default explorer, startup), so it is
safe to experiment with.

## Project layout

| Path | What lives there |
|---|---|
| `src/ui` | The interface (egui): views, tools, menus, dialogs |
| `src/fsx` | Windows file system and shell: listing, copying, thumbnails, archives, context menus |
| `src/index` | The search index, MFT/USN readers and the optional Windows service |
| `src/i18n.rs` | Translations: loading the CSV files and the `t!` / `tf!` / `tp!` macros |
| `idiomas/` | One CSV per language (`es.csv`, `en.csv`) |
| `vendor/winit` | winit 0.30.13 with one added function, see the README |

## Guidelines

- **Keep it fast.** Rayo redraws only when something changes and never blocks the UI thread on disk or
  network I/O. Slow work goes to a background thread and wakes the UI when it is done.
- **No user-facing text in code.** Use `t!("area.key")`, `tf!("area.key", n = 3)` or `tp!(count, "area.key")`
  (plurals: `area.key.uno` / `area.key.varios`) and add the text to both `idiomas/es.csv` and `idiomas/en.csv`.
  `cargo test` fails if a key is missing or its `{variables}` don't match.
- **Match the surrounding code**: naming, comment density and idioms. Most existing comments are in Spanish;
  new comments may be in English or Spanish.
- **Windows behavior matters.** If you change copying, deleting, renaming or anything that touches the
  registry, describe how you tested it (and, if possible, add a test that works in a temporary folder).
- **One topic per pull request**, with a clear description of what changed and why.

## Pull request checklist

- [ ] `cargo build --release` has no warnings
- [ ] `cargo test --lib` passes
- [ ] New interface text is in `idiomas/es.csv` and `idiomas/en.csv`
- [ ] Screenshots for visual changes

## Releasing (maintainers)

Releases are automatic. Bump `version` in `Cargo.toml`, commit, then push a matching tag:

```bash
git tag v0.2.0
git push origin v0.2.0
```

The [Release workflow](.github/workflows/release.yml) checks that the tag matches `Cargo.toml`, builds and
tests on Windows, and publishes `rayo.exe` with its SHA-256 and the list of changes since the previous release.

By contributing, you agree that your contributions are licensed under the [MIT License](LICENSE) and that you
follow our [Code of Conduct](CODE_OF_CONDUCT.md).
