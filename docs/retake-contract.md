# Retake analysis contract (`simplecut.retakes/1`)

This is the one file format between the retake-detection pipeline and the SimpleCut editor. **The pipeline produces it; the editor consumes it.** The editor owns the format: if something here is awkward for the pipeline, change it here first, then in both programs.

The editor's side is `crates/core/src/retake.rs` (parsing, validation, applying). A complete example is [`examples/retake-sample.json`](examples/retake-sample.json); a test parses it, so this document and the code cannot silently drift apart.

## What the editor does with it

1. Reads one document per media file (video or audio).
2. Checks it (see "Validation"). Proposals that fail are listed with the reason and ignored; a document made for a different file is refused as a whole.
3. Shows the proposals in a review list and on the timeline. **Nothing is cut until the user approves it.**
4. Cuts the approved ranges out of the timeline as one undo step. Picture and sound are cut together, the gap closes, music is left alone.

## The document

```json
{
  "schema": "simplecut.retakes/1",
  "media": { "path": "C:/videos/take1.mp4", "duration_seconds": 312.48 },
  "generator": { "name": "retake", "version": "0.3.0", "model": "whisperx-large-v2 + gemini-2.5-flash" },
  "transcript": { "language": "en", "words": [ { "text": "Hello", "start": 0.48, "end": 0.81, "confidence": 0.97 } ] },
  "proposals": [
    {
      "id": "span_3:0",
      "start": 12.34,
      "end": 15.80,
      "confidence": 0.93,
      "decision": "remove",
      "reason": "Same sentence restarted; the later take is complete.",
      "removed_text": "So today we're going to, um, so today we are going to",
      "kept_text": "So today we are going to talk about editing.",
      "kept_start": 15.80,
      "kept_end": 19.2,
      "source": "llm",
      "group": "span_3"
    }
  ]
}
```

### Required

| Field | Meaning |
|---|---|
| `schema` | Exactly `"simplecut.retakes/1"`. Anything else is refused. |
| `media.duration_seconds` | Length of the analysed file. The editor compares it with the file it has; more than 0.5 s apart means "made for a different file" and the whole document is refused. |
| `media.path` | Where the pipeline found the file. Informational only; the editor does not trust it. |
| `proposals[]` | May be empty (nothing found). |
| `proposals[].id` | Non-empty, unique in the document. Suggested: `"<span_id>:<n>"`. |
| `proposals[].start`, `.end` | **Seconds, as numbers, on the source file's own clock** (see below). `0 <= start < end <= media.duration_seconds`. |
| `proposals[].confidence` | Number from 0 to 1. |

### Optional (all have safe defaults)

| Field | Default | Meaning |
|---|---|---|
| `proposals[].decision` | `"remove"` | `"remove"`: the pipeline recommends cutting this (the editor pre-selects it when confidence is high). `"review"`: the pipeline is not sure or could not decide (the editor never pre-selects it). |
| `reason` | `""` | One or two sentences shown to the user. |
| `removed_text`, `kept_text` | `""` | What is cut and what stays instead, for the review list. |
| `kept_start`, `kept_end` | none | Where the kept take is, so the editor can jump to it. |
| `source` | none | `"stage2"`, `"llm"`, `"rule"`... free text, shown as a tag. |
| `group` | none | Proposals of the same retake group share it (for example the span id). |
| `generator`, `transcript` | none | Recorded with the result. The transcript is not used for cutting yet. |

Unknown fields are ignored, so the pipeline may add its own. Never reuse a field name for a different meaning; changing a meaning needs `.../2`.

### What "seconds on the source file's own clock" means

`0.0` is the first moment of the file as a player shows it, and times are in the units the editor uses for "where in the file" (`source_in`). The editor converts them to timeline positions itself, so it does not matter whether the clip was trimmed, moved or placed twice.

Be careful when the audio is extracted for transcription: if the audio stream starts later or earlier than the video (a container `start_time` offset), add that offset so times line up with the picture. Check this once with a real file and the sample cut, by listening at the cut points.

The editor rounds each cut to the nearest video frame. If the pipeline wants breathing room around a cut, add it to `start`/`end`; the editor cuts exactly what it is given.

### Confidence

Suggested mapping from the current pipeline (the editor does not care how the number was made, only that higher means "more sure it is a retake"):

| Pipeline result | `decision` | `confidence` | `source` |
|---|---|---|---|
| Span needed no language model (`not_required`) | `remove` | the span's best Stage 2 score | `stage2` |
| LLM resolved, genuine retake, confidence `high` | `remove` | 0.9 | `llm` |
| LLM resolved, genuine retake, confidence `medium` | `remove` | 0.7 | `llm` |
| LLM resolved, genuine retake, confidence `low` | `review` | 0.5 | `llm` |
| LLM failed after retries (`failed`) | `review` | 0.3 | `llm` |
| LLM says not a genuine retake | no proposal | | |

### Mapping from the current pipeline's own data

| This contract | The pipeline's data (see `repository_walkthrough.md`) |
|---|---|
| one proposal | one merged discard range of a span's `units_to_remove` (`cut_generator.py`, merge tolerance 0.05 s) |
| `start`, `end` | the removed units' `start` / `end` |
| `removed_text` | the removed units' `raw_text`, joined |
| `kept_text`, `kept_start`, `kept_end` | the span's `keeper_unit_id` unit |
| `reason` | the adjudication `reasoning` |
| `group` | `span_id` |

The pipeline's own FFmpeg editing step is not used; the editor makes the cuts.

## Validation (what the editor refuses, and what it merely drops)

Refused as a whole, with a message: wrong or missing `schema`; not valid JSON; `media.duration_seconds` missing, not a positive number, or more than 0.5 s different from the file; more than 100,000 proposals; larger than 64 MB.

Dropped one by one, each listed with its reason (never guessed at): empty or duplicate `id`; `start`/`end` not a finite number; `start >= end`; shorter than 10 ms; outside the file (an `end` up to 50 ms past the end is clamped); `confidence` not a number from 0 to 1.

## Applying

Approved proposals of one file are cut from the timeline's main tracks (Video and Dialogue). For each clip showing that file, the part of a proposal inside the clip's used range is removed; parts outside are ignored. Overlapping or touching cuts are merged. Clips that are speed-changed are skipped and reported. The result is one undo step.

## Versioning

`simplecut.retakes/1` is this document. A change that old readers cannot ignore gets a new number and the editor keeps reading the old one.
