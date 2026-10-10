# safeclean

Find build artifacts and dependency caches, pick what to delete from a
collapsible tree, and reclaim the disk space.

```bash
safeclean                  # scan current dir, pick interactively
safeclean ~/projects       # scan a specific path
safeclean -t rust,node     # only these project types
safeclean -n               # dry run: pick as usual, delete nothing
safeclean -y               # delete everything found, no questions
safeclean --fast           # estimate sizes by sampling big directories
safeclean --demo           # try the UI on simulated data
```

## Picking

Nothing is selected and everything is collapsed to start with. Select a whole
project type in one keystroke, or expand it and decide directory by directory.

| Key | Action |
| --- | --- |
| `↑` `↓` / `j` `k` | move |
| `→` / `l` | expand, then step into |
| `←` / `h` | collapse, then jump to parent |
| `space` | select / deselect the row and everything under it |
| `a` | select / deselect everything |
| `e` | expand / collapse everything |
| `tab` | switch between *by type* and *by directory* views |
| `enter` | delete the selection (asks for `y` first) |
| `q` / `esc` | quit without deleting |

Selections carry over between the two views.

## What it finds

A directory only counts when its name matches *and* the project marker is
present, so a stray `target/` or `build/` folder is left alone.

| Type (`-t`) | Directories | Marker |
| --- | --- | --- |
| `rust` | `target` | `Cargo.toml` |
| `node` | `node_modules` | |
| `web` | `.next` `.nuxt` `.svelte-kit` `.turbo` `.parcel-cache` `.angular` | framework config or `package.json` |
| `python` | `.venv` `venv` `__pycache__` `.pytest_cache` `.mypy_cache` `.ruff_cache` `.tox` | `pyvenv.cfg` for venvs |
| `maven` | `target` | `pom.xml` |
| `gradle` | `build` `.gradle` | `build.gradle[.kts]` |
| `dotnet` | `bin` `obj` | `*.csproj` / `*.fsproj` / `*.vbproj` |
| `zig` | `.zig-cache` `zig-cache` `zig-out` | `build.zig` |
| `elixir` | `_build` `deps` | `mix.exs` |
| `swift` | `.build` | `Package.swift` |
| `haskell` | `dist-newstyle` `.stack-work` | |
| `dart` | `.dart_tool` `build` | `pubspec.yaml` |
| `terraform` | `.terraform` | |

Sizes are disk usage (like `du`), symlinks are never followed, and the scan
stays on one filesystem. The walk runs on every core. `--fast` measures only a
sample of the files in large directories, which helps on a slow or cold disk at
the cost of individual sizes being off.

## Afterwards

The status line shows free space now and what it would become with the current
selection. Once the deletion is done you get the real before and after, as the
filesystem reports it:

```text
Freed 12.4 GB in 8 directories

  ██████████████████████████░░░░░░  1.0 TB disk
  free  182.0 GB → 194.4 GB
  used  82% → 81%
```
