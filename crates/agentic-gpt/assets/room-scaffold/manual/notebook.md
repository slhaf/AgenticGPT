# Room notebook maintenance

The `notebook` maintenance slot accepts one bounded document payload:

```json
{
  "path": "Notebook/topic.md",
  "title": "short title",
  "body": "Markdown body"
}
```

`path` must remain below `Notebook/`, use `/` separators, and end in `.md`; it is limited
to 240 characters. `title` is optional and at most 512 characters. `body` is required and
at most 64 KiB. The payload object has no other fields. The repository-owned
`scripts/apply_maintenance.py` validates these limits and writes only the requested document.
