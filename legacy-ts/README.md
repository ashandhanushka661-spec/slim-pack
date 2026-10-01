# legacy-ts — the TypeScript v0.1 implementation

This directory is where the TypeScript-only v0.1 implementation was supposed to
be moved, **untouched**.

It is empty, because the repository contains no TypeScript sources to move.
Verified:

```console
$ git ls-tree -r v0.1 --name-only
README.md

$ git show --stat v0.1
 README.md | 1 +
```

The `v0.1` tag (commit `96d62df`) contains exactly one file, a one-line
`README.md`, and no `package.json`, `src/` or TypeScript files at all. The git
history has not been rewritten and the tag has not been moved.

Nothing is deleted here. If a v0.1 TypeScript implementation exists somewhere
else (another repository, an unexported branch, a local working copy), drop it
into this directory unmodified and record the source in `PROGRESS.md`.