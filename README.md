# cjk-token-reducer
Reduce Claude Code token usage by 35-50% when using CJK languages.

## The Problem
CJK (Chinese, Japanese, Korean) languages consume 2-4x more tokens than English for the same semantic content.
This discrepancy leads to higher costs, faster context exhaustion, and reduced context windows for RAG/agent workflows.

| Language | Avg Token Ratio | Typical Range | Notes |
|----------|-----------------|---------------|-------|
| Chinese | ~2.0-3.0x | 1.5-4.0x | Rare characters may split into 3-4 tokens |
| Japanese | ~2.12x | 1.5-8.0x | Mixed Kanji/Kana creates segmentation challenges |
| Korean | ~2.36x | 2.0-3.0x | Agglutinative nature compounds inefficiency |

*Token ratios based on BPE tokenizer analysis. Actual savings depend on text complexity and technical term density.*

### Why Does This Happen?
The inefficiency stems from the mechanics of Byte-Pair Encoding (BPE) and training data distribution:
1. Vocabulary Bias: Modern tokenizers train primarily on English corpora.
   Common English words merge into single tokens.
   CJK characters, occurring less frequently in training data,
   often fail to merge into "words" and split into individual character tokens or raw bytes.
2. UTF-8 Byte Fallback: A common cause of token expansion.
   - Many LLM tokenizers process text as UTF-8 bytes.
   - An English character is 1 byte.
   - A CJK character is typically 3 bytes in UTF-8.
   - If a CJK character is absent from the tokenizer's vocabulary,
     byte-level tokenizers may expand it into multiple tokens.
     The exact expansion depends on the tokenizer's merge rules.
3. Lack of Delimiters: English uses spaces as natural word boundaries,
   aiding the tokenizer in identifying mergeable units.
   CJK languages lack these delimiters,
   forcing the tokenizer to rely purely on statistical frequency, which is lower for CJK sequences.

The consequence: API billing and model context limits measure tokens, not meaning.
Writing in CJK incurs a "tax" on both cost and memory.

## The Solution
CJK Token Reducer translates your CJK input to English before sending it to Claude.
English is the "native language" of LLM tokenizers, so translation acts as a compression layer.

### Key Features
- Reduces input token count by 35-50% (up to 2x effective context window)
- Preserves code blocks, file paths, and URLs (not sent for translation)
- Auto-detects English technical terms (camelCase, PascalCase, SCREAMING_SNAKE_CASE)
- macOS: Uses Apple NaturalLanguage framework for intelligent named entity recognition
- Caches translations locally to eliminate redundant API calls
- Uses free Google Translate API (no API key required)
- Sends only prompt text for translation; code artifacts stay local
- Adds 100-300ms latency per translation

### Trade-offs and Limitations
This tool implements a "Translate-Compute-Translate" (TCT) pattern.
While effective, it has inherent trade-offs:

| Aspect | Impact |
|--------|--------|
| Semantic Fidelity | Translation is lossy. Technical terms may shift meaning. Use `[[markers]]` to preserve critical terms. |
| Cultural Nuance | High-context CJK expressions may lose nuance when converted to English. |
| Latency | Adds 1-3 API calls. Suitable for async/batch workflows; less ideal for real-time chat. |
| Back-translation | Output translated back to CJK may sound unnatural ("translationese"). |

When NOT to use this tool:
- Precision-critical applications (legal, medical) where nuance matters
- Real-time chat requiring minimal latency
- When using native CJK-optimized models (DeepSeek V3, Qwen 2.5) which have efficient CJK tokenizers

Mitigation strategies:
- Use `[[term]]` markers to preserve technical terms from translation
- Enable `englishTerms` detection to auto-preserve English words in CJK text
- Create custom glossaries for domain-specific terminology (planned feature)

## Installation

### Option 1: Cargo Install (Recommended)
```shell
# Linux/Windows
cargo install --git https://github.com/jserv/cjk-token-reducer

# macOS (with NLP support)
cargo install --git https://github.com/jserv/cjk-token-reducer --features macos-nlp
```

### Option 2: Build from Source
```shell
git clone https://github.com/jserv/cjk-token-reducer
cd cjk-token-reducer

# Linux/Windows
cargo build --release

# macOS (with NLP support)
cargo build --release --features macos-nlp

cp target/release/cjk-token-reducer ~/.local/bin/
```

## Setup

### 1. Configure Claude Code Hook
Add the following to your Claude Code settings file (usually `~/.claude/settings.json`).
This hook intercepts your prompt before submission.

```json
{
  "hooks": {
    "UserPromptSubmit": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "cjk-token-reducer"
          }
        ]
      }
    ]
  }
}
```

The tool accepts JSON input `{"prompt": "..."}` on stdin and outputs modified JSON.

#### How It Works
The hook intercepts at `UserPromptSubmit`, translating CJK prompts before Claude processes them:

```
┌──────────────────────────────────────────────────────────────┐
│                      Claude Code Session                     │
├──────────────────────────────────────────────────────────────┤
│  SessionStart ─────► User types prompt (CJK)                 │
│                           │                                  │
│                           ▼                                  │
│              ┌────────────────────────────┐                  │
│              │    UserPromptSubmit        │                  │
│              │  ┌──────────────────────┐  │                  │
│              │  │  cjk-token-reducer   │  │ ◄─ Intercept     │
│              │  │  - Detect CJK        │  │                  │
│              │  │  - Check cache       │  │                  │
│              │  │  - Translate → EN    │  │                  │
│              │  │  - Preserve code     │  │                  │
│              │  └──────────────────────┘  │                  │
│              └────────────────────────────┘                  │
│                           │                                  │
│                           ▼                                  │
│                   Claude processes (English prompt)          │
│                           │                                  │
│                   ┌───────┴───────┐                          │
│                   ▼               ▼                          │
│              PreToolUse      (No tools)                      │
│                   │               │                          │
│                   ▼               │                          │
│              Tool executes        │                          │
│                   │               │                          │
│                   ▼               │                          │
│              PostToolUse          │                          │
│                   │               │                          │
│                   └───────┬───────┘                          │
│                           ▼                                  │
│                        Stop                                  │
└──────────────────────────────────────────────────────────────┘
```

### 2. Configuration (Optional)
Create a `.cjk-token.json` file to customize behavior.
The tool searches these locations in order:

1. Current directory: `./.cjk-token.json`
2. Home directory: `~/.cjk-token.json`
3. Config directory: `~/.config/cjk-token-reducer/.cjk-token.json`

```json
{
  "outputLanguage": "en",
  "threshold": 0.1,
  "enableStats": true,
  "cache": {
    "enabled": true,
    "ttlDays": 30,
    "maxSizeMb": 10
  },
  "preserve": {
    "englishTerms": true,
    "useNlp": true
  }
}
```

#### Configuration Options
| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `outputLanguage` | string | `"en"` | Desired response language from Claude. See below. |
| `threshold` | number | `0.1` | Ratio of CJK characters required to trigger translation (0.1 = 10%). |
| `enableStats` | boolean | `true` | Track and save token usage statistics. |
| `cache.enabled` | boolean | `true` | Enable translation caching to reduce API calls. |
| `cache.ttlDays` | number | `30` | Cache entry time-to-live in days. |
| `cache.maxSizeMb` | number | `10` | Maximum cache size in megabytes. |
| `preserve.englishTerms` | boolean | `true` | Auto-detect and preserve English technical terms in CJK text. |
| `preserve.useNlp` | boolean | `true` | Use macOS NLP for named entity detection (macOS only, falls back to regex). |

#### Data Storage Locations
The tool stores translation cache and statistics in platform-specific directories:

| Platform | Cache Directory | Statistics Directory |
|----------|-----------------|---------------------|
| Linux | `~/.cache/cjk-token-reducer/` | `~/.config/cjk-token-reducer/` |
| macOS | `~/Library/Caches/cjk-token-reducer/` | `~/Library/Application Support/cjk-token-reducer/` |
| Windows | `%LOCALAPPDATA%\cjk-token-reducer\` | `%APPDATA%\cjk-token-reducer\` |

Files within these directories:
- `translations.db/` — sled embedded database for translation cache
- `stats.json` — token usage statistics

#### Output Language Settings
- `"en"` (default): Claude responds in English.
  This yields maximum token savings for both input and output.
- `"zh"`, `"ja"`, `"ko"`: Instructs Claude to reply in the specified language.
  Saves input tokens, but output remains in CJK and consumes more tokens than English output.

#### Platform-Specific Features

**macOS NLP Integration**

On macOS, the tool leverages Apple's NaturalLanguage framework for intelligent named entity recognition.
This provides ML-based detection of:

| Entity Type | Examples | Benefit |
|-------------|----------|---------|
| Personal Names | Tim Cook, Elon Musk, Satya Nadella | Preserved without translation corruption |
| Place Names | Silicon Valley, Tokyo, Seoul | Geographic terms stay intact |
| Organization Names | Apple, Microsoft, Google | Company names remain recognizable |

The NLP detector also supports extended Latin characters (e.g., "Rene", "Munchen", "Francois")
while correctly filtering out CJK names that should be translated.

**Comparison: NLP vs Regex Detection**

| Aspect | Regex (All Platforms) | NLP (macOS) |
|--------|----------------------|-------------|
| Technical Terms | camelCase, PascalCase, SNAKE_CASE | Same + named entities |
| Proper Names | Only if capitalized patterns match | ML-based recognition |
| Context Awareness | Pattern-based only | Semantic understanding |
| Performance | Faster | ~10-50ms overhead per call |

To disable NLP and use regex-only detection on macOS:
```json
{
  "preserve": {
    "useNlp": false
  }
}
```

## Usage
Once installed and configured, use Claude Code normally.

```shell
claude
❯ 重構這個函式
# Automatically translated to: "Refactor this function"

❯ この関数をリファクタリングしてください
# Automatically translated to: "Please refactor this function"

❯ 이 함수 리팩토링 해줘
# Automatically translated to: "Refactor this function"
```

### CLI Commands
```shell
# View token savings statistics
cjk-token-reducer --stats

# View cache statistics
cjk-token-reducer --cache-stats

# Clear translation cache
cjk-token-reducer --clear-cache

# Preview translation without sending (dry run)
cjk-token-reducer --dry-run

# Bypass cache for single translation
cjk-token-reducer --no-cache
```

### Viewing Statistics
Track your token savings over time:

```shell
cjk-token-reducer --stats
```

Output example:
```text
╔══════════════════════════════════════════════════════════╗
║           CJK Token Reducer Statistics                   ║
╠══════════════════════════════════════════════════════════╣
║  Total Translations:            150                      ║
║  Translation Tokens:           3200                      ║
║  Estimated Saved:              8500                      ║
╚══════════════════════════════════════════════════════════╝
```

## Privacy & Security
- Translation Service: This tool uses the public Google Translate API.
  Your text prompts are sent to Google's servers.
- Code Security: The tool preserves code blocks and file paths locally,
  preventing them from being sent to the translation service.
- Data Handling: No data is stored by this tool other than local usage statistics (if enabled) and translation cache.

## Development
```shell
# Build (Linux/Windows)
cargo build

# Build with NLP support (macOS only)
cargo build --features macos-nlp

# Run tests
cargo test

# Run tests with NLP (macOS only)
cargo test --features macos-nlp

# Build for release (macOS with NLP)
cargo build --release --features macos-nlp
```

## Alternatives
For preserving original language while reducing tokens,
consider [LLMLingua](https://github.com/microsoft/LLMLingua) — Microsoft's perplexity-based compression toolkit.

How LLMLingua Works:
1. Uses a small LM (GPT-2 or LLaMA-7B) to compute token perplexity
2. Removes tokens with low information content (high predictability)
3. Preserves critical tokens that carry semantic weight

When to Choose LLMLingua:
- You need to preserve the original CJK language in prompts
- Working with very long contexts (RAG, document Q&A)
- Compression ratio is more important than perfect fidelity
- Integrated workflows (LangChain, LlamaIndex, Prompt Flow)

When to Choose cjk-token-reducer:
- Daily Claude Code usage with CJK input
- Simplicity over maximum compression
- No additional model inference overhead
- Code-heavy prompts (LLMLingua may corrupt code blocks)

## License
`cjk-token-reducer` is available under a permissive MIT-style license.
Use of this source code is governed by a MIT license that can be found in the [LICENSE](LICENSE) file.

## References
* Petrov et al. (2023): [Language Model Tokenizers Introduce Unfairness Between Languages](https://arxiv.org/abs/2305.15425) - Analysis of tokenization disparity, showing up to 15x inefficiency for some languages.
* Ahia et al. (2023): [Do All Languages Cost the Same? Tokenization in the Era of Commercial Language Models](https://arxiv.org/abs/2305.13704) - Examines cost implications of tokenizer design on non-English languages.
* Yennie Jun: [All Languages Are NOT Created (Tokenized) Equal](https://www.artfish.ai/p/all-languages-are-not-created-tokenized) - Visualizations and statistics on cross-lingual tokenization efficiency.
