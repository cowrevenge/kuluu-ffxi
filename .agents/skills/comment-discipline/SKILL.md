---
name: comment-discipline
description: The code-comment rules for this repository and how to self-check them. Use before writing or reviewing any Rust comment, doc comment, citation to vendor/research sources, or hex/numeric literal, and when a commit or push is rejected by scripts/checks.sh comments. Applies in every harness, including ones without this repo's hook adapters (LM Studio Bionic, Cursor, Aider).
---

# Comment discipline

Default: no comment. Names, types, and asserts carry WHAT and HOW. Write a
comment only when it is one of three things:

1. A WHY the code cannot encode (a tuning the data cannot supply, a
   non-obvious ordering constraint).
2. A citation to a source someone can open from this tree: `vendor/...`
   (LSB, POLUtils), `research/...` (XIClient, xim, XiPackets, cexi-docs), a
   retail binary symbol/RVA, or an observation record under
   `.agents/skills/retail-observe/references/`.
3. A `// SAFETY:` justification on an `unsafe` block.

Doc comments (`///`, `//!`) are held to the same bar: one tight, accurate
sentence beats a paragraph that will rot.

## Citations

- Anchor on a symbol, never a line number. Line numbers decay on the next
  submodule bump and nothing catches it.
  - bad: `vendor/server/src/map/attack.h:52-59`
  - good: `vendor/server/src/map/attack.h AttackAnimation`
  - bad: `research/xim EffectRoutineParser.kt:400`
  - good: `research/xim EffectRoutineParser.kt parseStopRoutine`
- The path must exist in this tree. Never cite a private note, a finding id
  from someone's notebook (`(F37, F47)`), a retired `docs/` file, or an
  elided path (`research/XIClient/.../Foo.cpp`). If the evidence lives
  outside the repo, first land it as a dated record under
  `.agents/skills/retail-observe/references/` and cite that.
- A retail-binary citation looks like `FFXiMain.dll .data @RVA 0x35AF60`
  plus the client version observed.

## Literals

A number that carries meaning gets a named `const`, never an inline value
and never a comment explaining the value. If it derives from LSB/POLUtils,
scrape it at build time (see the `vendor-scrape` skill). If it is a
deliberate tuning, name the const and cite the WHY in one line.

- bad: `a + if a < 0x200 { 0x0F3C } else { 0xC1EF }`
- good: `const MOB_SKILL_FILE_TABLE_OFFSET_LOW: u32 = 0x0F3C;` with a
  `research/xim MobAbilityTable.kt getFileTableOffset` citation, then
  `a + MOB_SKILL_FILE_TABLE_OFFSET_LOW`.
- A wire tag or text marker one module emits and another matches is an
  exported `const` on the emitter, imported by the consumer, pinned by a
  guard test.

## Prune-by-default families

Delete or rewrite on sight: session history ("previously", "no longer",
"for now", "stage 2", "this replaces"), commented-out code, markdown
decoration in `//` comments, a claim like "always"/"never" that no assert or
type enforces, and a hex literal restated in prose next to the code that
already contains it. git log is the change history; describe the code as it
is.

## Self-check

```bash
scripts/checks.sh comments                       # whole tree + advisory diff vs origin/main
COMMENTS_DIFF=staged scripts/checks.sh comments  # only what you are about to commit
```

Hard failures: a line-pinned citation, a cited path that does not exist, an
elided path, a finding id. Everything else prints as advisory; read it and
decide, then commit. `.githooks/pre-commit` runs the staged form on every
commit once hooks are installed (`cargo xtask install-hooks`); pre-push and
CI run the tree form.

## Plans and handoffs

A plan that will be executed by an agent inherits these rules: its code
snippets must already cite symbols, name constants, and point at in-tree
evidence. "Add a comment explaining X" in a plan is a smell; the fix is a
name, a type, or a test.
