---
name: wexport implementation
description: Implement, fix or review code in the wexport repository (Android WhatsApp crypt15 backup → offline chat archive). Use for any edit under src/ or tests/, for milestone work, and for judging whether a change respects the project's invariants.
---

# Implementing wexport

## The spec is the plan, this is the procedure

`docs/implementation-plan.md` is the specification. This skill is the procedure and the quality
bar. When the two disagree, the plan wins and this file is the thing to fix. Never copy spec text
here: point at the section.

Read before editing:

1. `docs/implementation-plan.md` — the section covering the stage you touch (§2 pipeline, §4 data
   model, §5 media, §6 archive format, §7 milestones).
2. `docs/android-backup-formats.md` — only for decryption, keys, msgstore schema, incrementals.
3. `docs/user-flow.md` — only when the change is observable by the person using the tool.

## Invariants a change may never break

| Invariant | Why it matters | Spec |
|---|---|---|
| Only `db/msgstore.py` knows WhatsApp table or column names | one file to fix when WhatsApp renames something; everything else survives | §3 |
| The archive opens from `file://`; the product has no server | the recipient must need nothing installed, not even wexport | §6 |
| Timestamps are frozen at build; the viewer never derives a local time | timezone rules change; the archive must keep saying when it happened | §4 |
| Media are content-addressed inside the chat folder, never grouped by chat on disk | a chat folder stands alone when copied or zipped | §5 |
| A chat archive never reveals that other chats exist | sharing one conversation must not disclose the rest | §6 |
| Exit codes `0..5` are the interface; each has a test | the CLI is what the person actually drives | §2 |
| A missing external binary aborts; nothing degrades silently | an archive without thumbnails looks finished and is not | §2 |

## How to work

One unit at a time — a module and the test that proves it — then commit. A unit is a milestone
bullet, not a whole milestone.

1. Pick the next milestone (§7) and the smallest unit inside it.
2. Write the test first against the **synthetic fixture**; if the fixture lacks the shape the test
   needs, extend the fixture generator (`references/fixtures.md`), not the assertion.
3. Write the code in the file named by the tree in §3. If the file does not exist and the tree does
   not name it, justify it in the commit message or do not create it.
4. Re-read the diff. Does it duplicate something already present? Does it work around a constraint
   instead of satisfying it? Could it be smaller? Answer honestly and cut.
5. Commit, one unit per commit, message in English imperative:
   `feat(db): read message_add_on reactions into chat.db`, `fix(media): keep peaks for zero-length
   voice notes`.

Tests are written but **not run on your own initiative** (owner rule): the project's checks run
once, whole tree, before a push, and the owner decides when a push happens. Reading the code and
the diff is what judges a unit.

## Never do this

- Add a dependency to avoid writing thirty lines. `wa-crypt-tools`, `Pillow`, `Nuitka` and stdlib
  are the budget; anything else needs the owner.
- Reintroduce `fetch`, ES modules, Web Workers or `localStorage` in the viewer because they feel
  convenient. They are exactly what `file://` blocks (§6).
- Recompute in the viewer anything the build already froze (timestamps, dimensions, mime, paths).
- Put a chat list, an index or a link to another chat inside a chat archive.
- Probe WhatsApp's version string to decide a schema shape when the schema itself answers.
- Make a fixture match the code instead of the reality the fixture is meant to represent.
- Silence a failure into a warning. Either the archive is correct or the exit code says why not.
- Read a WhatsApp column anywhere outside `db/msgstore.py`.

## Adding a message kind

Three places, in this order: the mapping in `db/msgstore.py` → the `kind` vocabulary in §4 (with the
plan updated in the same commit) → a renderer in `app.js` with a fallback for unknown payloads. If
the kind needs a new data field, it goes in `payload` JSON; a new column requires a schema change and
a reason in the commit.

## Definition of done for a unit

- Code in the planned location, no dead branches, no TODO without a `shortcut:` note.
- A test that fails if the behaviour regresses (synthetic fixture, or `WEXPORT_REAL_BACKUP` when
  only real data can prove it — and the test says why in its docstring).
- Docs updated only where observable behaviour changed: §4/§5/§6 of the plan, or `docs/user-flow.md`
  when the user sees it.
- `git status` clean, one commit, message that names the unit.
