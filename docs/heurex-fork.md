# Sentrux Heurex Fork

This repository is the Heurex fork of Sentrux. The fork is intended to be easy
to distinguish from the upstream `sentrux/sentrux` release in command output,
release names, and Windows executable metadata.

## Version Identity

Current fork release: `0.5.15`.

The CLI version string includes the fork stamp:

```text
sentrux 0.5.15 (Heurex fork)
```

On Windows, the executable embeds a VERSIONINFO resource with:

- `FileDescription`: `Sentrux Heurex fork`
- `ProductName`: `Sentrux Heurex fork`
- `FileVersion`: `0.5.15.0 (Heurex fork)`
- `ProductVersion`: `0.5.15-heurex-fork`
- `PrivateBuild`: `Heurex fork`

The Windows stamp is generated from `CARGO_PKG_VERSION` in
`sentrux-bin/build.rs`, so a Cargo package patch bump updates the executable
stamp automatically.

## Agent Debug Commands

For a failed gate, agents should start with:

```bash
sentrux plugin verify --json --plugin-root <plugins> --require-language csharp
sentrux gate --json --include-untracked --plugin-root <plugins> --require-language csharp <repo>
```

This compares the current scan against `.sentrux/baseline.json` and emits the
before/after metric counts, hard degradations, and added/removed/current
offenders for actionable metrics.

`--plugin-root` should point at a provisioned plugin directory. Sentrux verifies
that directory and reports its inventory; it does not download, repair, or
install plugins during `check`, `gate`, or `plugin verify`. Praxis/scaffold
provisioning owns download and installation.

Use `--include-untracked` when debugging pre-commit gate failures or brand-new
files that have not been `git add`-ed yet. Without the flag, `gate` preserves
the existing tracked-file-only behavior.

For a current structural assessment that includes untracked worktree files,
agents should run:

```bash
sentrux check --json --include-untracked --plugin-root <plugins> --require-language csharp <repo>
```

This reports the current rules result, scan options, C# reference summary, and
current metric offender lists even when the check passes.

## Actionable Diagnostics

The fork changes Sentrux reporting from aggregate counters to root-cause
diagnostics. The most important JSON paths are:

- `gate --json`: `degradations[]`
- `gate --json`: `analysis.complete`
- `gate --json`: `analysis.fatalDiagnostics[]`
- `gate --json`: `analysis.inventory.languages[]`
- `gate --json`: `analysis.structuralCoverage.requiredLanguages[]`
- `gate --json`: `analysis.structuralCoverage.unparsedCodeFiles[]`
- `gate --json`: `scan.include_untracked`
- `gate --json`: `hardMetricFailureDespiteQualityImprovement`
- `gate --json`: `metrics.godFiles.addedGodFiles[]`
- `gate --json`: `metrics.godFiles.removedGodFiles[]`
- `gate --json`: `metrics.godFiles.persistingGodFiles[]`
- `gate --json`: `metrics.godFiles.changedRankOrScoreGodFiles[]`
- `gate --json`: `metrics.coupling.offenders.added[]`
- `gate --json`: `metrics.cycles.cycles.added[]`
- `check --json`: `metrics.godFiles.files[]`
- `check --json`: `analysis.complete`
- `check --json`: `analysis.fatalDiagnostics[]`
- `check --json`: `analysis.inventory.languages[]`
- `check --json`: `analysis.structuralCoverage.requiredLanguages[]`
- `check --json`: `analysis.structuralCoverage.unparsedCodeFiles[]`
- `check --json`: `metrics.coupling.problemEdges[]`
- `check --json`: `metrics.cycles.cycles[]`
- `check --json`: `metrics.depth.deepestFiles[]`
- `check --json`: `metrics.complexFunctions.functions[]`
- `check --json`: `metrics.longFunctions.functions[]`
- `check --json`: `metrics.largeFiles.files[]`
- `check --json`: `metrics.duplicates.groups[]`
- `check --json`: `metrics.deadFunctions.functions[]`

God-file records include the repo-relative path, language, reason,
score/threshold, LOC, imports, fan-in, fan-out, call edges, centrality,
coupling, and complexity where Sentrux can compute them.

Cycle records include edge chains. Each edge can carry the source file, target
file, edge kind, symbol or type name, line/column, resolver source, and whether
the edge came from normal imports, project references, C# type references, call
inference, or resolver fallback.

The text output mirrors the JSON RCA for operators who are reading logs.

Stable fatal diagnostic codes include:

- `SENTRUX-LANGUAGE-PLUGIN-MISSING`
- `SENTRUX-PLUGIN-MANIFEST-MISSING`
- `SENTRUX-PLUGIN-MANIFEST-INVALID`
- `SENTRUX-GRAMMAR-MISSING`
- `SENTRUX-GRAMMAR-CHECKSUM-MISSING`
- `SENTRUX-GRAMMAR-CHECKSUM-MISMATCH`
- `SENTRUX-GRAMMAR-LOAD-FAILED`
- `SENTRUX-GRAMMAR-ABI-INCOMPATIBLE`
- `SENTRUX-QUERY-MISSING`
- `SENTRUX-QUERY-INVALID`
- `SENTRUX-STRUCTURAL-COVERAGE-INCOMPLETE`
- `SENTRUX-GIT-UNTRACKED-ENUM-FAILED`

## Changelog

### Unreleased

- No unreleased fork changes.

### 0.5.15

- Linux ARM64 grammar release builds now install the cross C++ compiler needed
  for grammars with C++ scanners, including Nim.

### 0.5.14

- `check`, `gate`, and `plugin verify` now support immutable plugin-root
  verification with `--plugin-root` and repeatable `--require-language`.
- Required language plugins must be present, include a checksum for the current
  platform grammar, pass SHA-256 verification, load successfully, and compile
  their tree-sitter query. Failures are emitted under
  `analysis.fatalDiagnostics[]` with stable diagnostic codes.
- `check --json` and `gate --json` include an `analysis` envelope with plugin
  root source, inventory, required languages, structural coverage, fatal
  diagnostics, warnings, and explicit mutation policy.
- `gate` fails closed and refuses to save or compare a baseline when required
  structural coverage is incomplete.
- `--include-untracked` now fails closed if Git untracked-file enumeration
  fails instead of silently treating the untracked set as empty.
- Grammar release bundles now include plugin manifests, queries, a grammar
  release manifest, and per-platform SHA-256 checksum entries; the workflow
  fails if any supported platform bundle is missing or partial.

### 0.5.13

- Release downloads are built to avoid separate VC++ runtime, Homebrew/OpenSSL,
  or Linux GTK package requirements. The release workflow statically links the
  Windows CRT, vendors OpenSSL on Unix, uses the Linux portal file-dialog
  backend instead of direct GTK linkage, and fails the build if those runtime
  dependencies reappear.

### 0.5.12

- `gate` now accepts `--include-untracked` to include untracked working-tree
  files in the regression scan (default off; backward-compatible).

## Release Workflow

Patch release checklist for this fork:

1. Bump `sentrux-bin/Cargo.toml` and `sentrux-core/Cargo.toml`.
2. Confirm `Cargo.lock` contains the same package versions.
3. Run `cargo test --locked --workspace`.
4. Build the release binary and confirm `sentrux --version` includes
   `(Heurex fork)`.
5. Verify self-contained runtime behavior before release:
   - On Windows, run the forked executable in Windows Sandbox and confirm it
     starts without a VC++ Redistributable registry key being present.
   - On Linux, run the release binary in a fresh container and confirm `ldd`
     does not list OpenSSL, GTK, GDK, GLib, or GObject runtime packages.
6. Commit the change on `main`.
7. Push `main` to `origin`.
8. Tag `v<version>` and push the tag.
9. Confirm the grammar workflow attaches the full matrix:
   `darwin-arm64`, `darwin-x86_64`, `linux-x86_64`, `linux-aarch64`, and
   `windows-x86_64`.
10. The GitHub `Release` workflow builds fork-named release artifacts and names
   the release `Sentrux v<version> (Heurex fork)`.

Do not hard-code local release directories in repository files. Local tool
refresh paths, such as a personal OneDrive tools directory, belong in the
operator's release procedure outside the repo.
