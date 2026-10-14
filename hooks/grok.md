# Grok CLI (Grok Build)

## Configure Grok CLI hook
Grok CLI uses the same `UserPromptSubmit` hook format as Claude Code: JSON on
stdin, JSON on stdout, with a `prompt` field that can be replaced outright.

Add to `~/.grok/hooks/cjk-token-reducer.json` (or your project's
`.grok/hooks/`):

```json
{
  "hooks": {
    "UserPromptSubmit": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "/path/to/cjk-token-reducer"
          }
        ]
      }
    ]
  }
}
```

## How It Works
Same flow as Claude Code: the hook receives `{"prompt": "..."}` on stdin
before the prompt reaches the model, replaces CJK text with its English
translation (preserving code/URLs/paths), and returns `{"prompt": "..."}`
on stdout. No wrapper script is needed since Grok CLI speaks this format
natively.
