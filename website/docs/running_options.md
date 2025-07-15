---
sidebar_position: 5
---

# Debugger arguments overview

## View available arguments

To see all available command-line arguments:

```shell
bs --help
```

## Arguments

- `--tui` - start debugger with terminal UI (see [tui](/capabilities/tui.mdx)
- `--pid` (`-p`) <process_id> - attach to running process
- `--std-lib-path` (`-s`) <path> - specify a custom path to Rust standard library (when using non-default location)
- `--oracle` (`-o`) - discover a specific oracles (see [tui](/capabilities/oracle.mdx)
- `--theme` (`-t`) - set color theme for code visualization. Available themes:
  - none
  - inspired_github
  - solarized_dark (default)
  - solarized_light
  - base16_eighties_dark
  - base16_mocha_dark
  - base16_ocean_dark
  - base16_ocean_light
- `--keymap-file` (env: KEYMAP_FILE=) - path to TUI keymap file (default: ~/.config/bs/keymap.toml)
- `--save-history` (env: SAVE_HISTORY=) - retain command history between sessions
- `--version` (`-V`) - print version information
