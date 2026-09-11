# Room entity maintenance

The `entity` maintenance slot accepts one bounded entity payload:

```json
{
  "entity": "project",
  "content": "# Project\n\nCurrent state."
}
```

`entity` is one path component (at most 160 characters) and `content` is required and at
most 64 KiB. Separators, dot components, NUL bytes, and unknown fields are rejected. The
repository-owned `scripts/apply_maintenance.py` validates the payload and writes
`State/entities/<entity>.md` without following symlinks.
