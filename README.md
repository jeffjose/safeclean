# safeclean

Find build artifacts, caches and temp files, pick what to delete from a
collapsible tree, and reclaim the disk space.

```bash
safeclean                  # current dir plus caches, temp files and trash
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
| `s` | sort by size or by age, stalest first |
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

### Around the machine

Run with no directory and safeclean also looks in the usual places disposable
data piles up. Give it a directory and it sticks to that directory; add `-s`
to bring these back, or `-S` to leave them out of a bare run. `-y` never
includes them unless you pass `-s`.

| Type (`-t`) | Where | Offered as |
| --- | --- | --- |
| `cache` | `~/.cache` (or `$XDG_CACHE_HOME`), `~/Library/Caches` | one item per entry |
| `packages` | `~/.npm/_cacache` `~/.bun/install/cache` `~/.local/share/pnpm/store` `~/.cargo/registry` `~/.cargo/git` `~/.gradle/caches` `~/.m2/repository` `~/.nuget/packages` `~/.pub-cache` `~/go/pkg/mod` | one item each |
| `temp` | `/tmp`, `/var/tmp`, `$TMPDIR` | one item per entry you own |
| `trash` | `~/.local/share/Trash`, `~/.Trash` | the trash |

Sockets, pipes and symlinks in temp directories are never offered, but a
running program may still be using a file or directory there. That is what the
age column is for.

## How stale is it

Every row shows when it was last touched, and `s` re-sorts stalest first. For
a project that means the project, not the artifact: the newest change among
its files, its top-level directories and their contents, which takes in
`.git/index` and the build output itself. So both an edit and a build count as
activity. Recent rows are yellow, ones idle for 90 days or more are green.

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
