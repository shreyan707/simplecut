# Retake contract `simplecut.retakes/2`

What the editor and the retake service (the owner's Python pipeline: transcribe, split into
sentences, trained cross-encoder, grouping) say to each other. Examples are in `docs/examples/`.

**The service only finds and groups sentences. It never says what to keep and never calls an LLM.**
The user decides what to keep. An LLM is an optional extra, reached only by the "Ask AI to choose"
button.

Times are seconds on the media file's own clock. Sentence ids (`<unit>:<n>`) and pair ids
(`<a>|<b>`) are stable for the same file, so answers can be matched again later.

## Flow

1. `POST /v1/analyze` -> **candidates** (sentences and pairs). With nothing to ask, the service may
   answer with **groups** at once.
2. The editor shows the `ambiguous` pairs; the user marks each *same line* / *different lines* /
   skip, then presses **Next**. The answers are appended to a local `labels.jsonl` first.
3. `POST /v1/resolve` with the **labels** -> the cross-encoder makes **groups**.
4. The user ticks the sentences to **keep**; everything not kept is cut ("Remove"), one undo step.
5. Optional: `GET /v1/capabilities` says `{ "adjudicate": true }`; only then the button exists.
   `POST /v1/adjudicate` -> advice per group. AI advice is never sent back as training labels.

Calls are background jobs (progress, cancel).

## Documents

Every document has `schema: "simplecut.retakes/2"`, `stage`, `job`, `media {path, duration_seconds}`
and `generator {name, version, model}`.

**candidates** (`docs/examples/retake-candidates.json`): `sentences[ {id,start,end,text,confidence?} ]`,
`pairs[ {id,a,b,class:"likely"|"ambiguous",score} ]`.

**labels request** (`retake-labels.json`): `{job, labels:[{pair,label:"retake"|"different",source:"user"}], skipped:[pair ids]}`.
Optionally the whole candidates document, so a restarted service can recover.

**groups** (`retake-groups.json`): `groups[ {id,start,end,likely,sentences[...]} ]`.
`start`/`end` cover the whole group (for the highlight); `likely: true` groups are listed first and
expanded. No pairs, no keeper, no notes, no scores.

**advice** (`retake-adjudication.json`): `[ {group, keep:[sentence ids], reason} ]`.

## Validation (never guess)

- A document for another file or another `job` is refused.
- A group is dropped, with the reason shown, if it has fewer than two sentences, duplicate or
  unknown ids, non-finite or reversed times, times outside the file, or a span that does not cover
  its sentences (10 ms tolerance).
- Advice is checked the same way: an unknown id, or a reply that keeps nothing, is ignored for that
  group and noted.
- Documents over 20 MB are refused.

## In the editor

Every sentence has a keep tick; all start kept, so nothing is cut until the user chooses.
A group with fewer than two sentences left on the timeline is shown as "already handled" (Undo
brings it back). Files made by hand can be opened with Ctrl+Shift+R (candidates or groups).
The old `/1` format is no longer read.
