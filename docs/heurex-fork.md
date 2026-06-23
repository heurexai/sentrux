# Sentrux Heurex Fork

This repository is the Heurex fork of Sentrux. The fork is intended to be easy
to distinguish from the upstream `sentrux/sentrux` release in command output,
release names, and Windows executable metadata.

## Version Identity

Current fork release: `0.5.11`.

The CLI version string includes the fork stamp:

```text
sentrux 0.5.11 (Heurex fork)
```

On Windows, the executable embeds a VERSIONINFO resource with:

- `FileDescription`: `Sentrux Heurex fork`
- `ProductName`: `Sentrux Heurex fork`
- `FileVersion`: `0.5.11.0 (Heurex fork)`
- `ProductVersion`: `0.5.11-heurex-fork`
- `PrivateBuild`: `Heurex fork`

The Windows stamp is generated from `CARGO_PKG_VERSION` in
`sentrux-bin/build.rs`, so a Cargo package patch bump updates the executable
stamp automatically.

## Agent Debug Commands

For a failed gate, agents should start with:

```bash
sentrux gate --json <repo>
```

This compares the current scan against `.sentrux/baseline.json` and emits the
before/after metric counts, hard degradations, and added/removed/current
offenders for actionable metrics.

For a current structural assessment that includes untracked worktree files,
agents should run:

```bash
sentrux check --json --include-untracked <repo>
```

This reports the current rules result, scan options, C# reference summary, and
current metric offender lists even when the check passes.

## Actionable Diagnostics

The fork changes Sentrux reporting from aggregate counters to root-cause
diagnostics. The most important JSON paths are:

- `gate --json`: `degradations[]`
- `gate --json`: `hardMetricFailureDespiteQualityImprovement`
- `gate --json`: `metrics.godFiles.addedGodFiles[]`
- `gate --json`: `metrics.godFiles.removedGodFiles[]`
- `gate --json`: `metrics.godFiles.persistingGodFiles[]`
- `gate --json`: `metrics.godFiles.changedRankOrScoreGodFiles[]`
- `gate --json`: `metrics.coupling.offenders.added[]`
- `gate --json`: `metrics.cycles.cycles.added[]`
- `check --json`: `metrics.godFiles.files[]`
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

## Release Workflow

Patch release checklist for this fork:

1. Bump `sentrux-bin/Cargo.toml` and `sentrux-core/Cargo.toml`.
2. Confirm `Cargo.lock` contains the same package versions.
3. Run `cargo test --locked --workspace`.
4. Build the release binary and confirm `sentrux --version` includes
   `(Heurex fork)`.
5. Commit the change on `main`.
6. Push `main` to `origin`.
7. Tag `v<version>` and push the tag.
8. The GitHub `Release` workflow builds fork-named release artifacts and names
   the release `Sentrux v<version> (Heurex fork)`.

Do not hard-code local release directories in repository files. Local tool
refresh paths, such as a personal OneDrive tools directory, belong in the
operator's release procedure outside the repo.
