# Room diary maintenance

Diary maintenance requests are JSON objects no larger than 64 KiB. The slot selects the
canonical current document and the executor writes only bounded semantic fields.

For `diary.daily`, `diary.weekly`, and `diary.monthly` the payload is:

```json
{
  "summary": "optional string, at most 8,192 characters",
  "entries": [
    {"text": "string, at most 8,192 characters", "tags": ["short-tag"]}
  ]
}
```

`summary` is optional and `entries` contains at most 128 objects. Each entry may contain
only `text` and `tags`; there are at most eight tags per entry and each tag is at most 64
characters. Unknown fields are rejected. The repository-owned `scripts/apply_maintenance.py`
performs the semantic validation and writes the current Markdown file for the requested slot.
