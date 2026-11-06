# sidekick

An AI coding harness that doesn't write any code for you!

The purpose of `sidekick` is to help **you** write better code. Unheard of these days, but some
people want to write code by hand still :D.

`sidekick` watches your files for saves and tries to come up with suggestions for you on how to
simplify or make something better. **You** still have to fix it. You also can't ask `sidekick` any
questions, so if you need to research something, DuckDuckGo it (or Google, or Bing, or read a
book?)!

## Install

```sh
git clone git@github.com:omfj/sidekick && cd sidekick
cargo install --path .
```

## Setup

`sidekick` works with any OpenAI-compatible API. I use [OpenRouter](https://openrouter.ai), so that
is the default, but you are free to use your own proxy (local model maybe?).

```sh
export SIDEKICK_API_KEY=sk-or-...
```

Then start it in the project you're working on:

```sh
cd my-project
sidekick
```

### Options

| Flag           | Env                   | Default                        |
| -------------- | --------------------- | ------------------------------ |
| `--model`      | `SIDEKICK_MODEL`      | `anthropic/claude-opus-5.5`    |
| `--base-url`   | `SIDEKICK_BASE_URL`   | `https://openrouter.ai/api/v1` |
| `--effort`     | `SIDEKICK_EFFORT`     | provider default               |
| `--idle`       | `SIDEKICK_IDLE`       | `10s`                          |
| `--max-wait`   | `SIDEKICK_MAX_WAIT`   | `1m`                           |
| `--cost-every` | `SIDEKICK_COST_EVERY` | `10m` (`0` = off)              |
| `--notify`     | `SIDEKICK_NOTIFY`     | off                            |

`--notify` sends a desktop notification through your terminal with OSC 777 (same as Ghostty).

Durations take `s`/`sec`/`secs` or `m`/`min`/`mins`, e.g. `30s` or `5mins`. Saves are batched: a
review runs once you've stopped saving for `--idle`.

### Local setup

To use your local provider, set `--base-url`. Local servers need no key:

```sh
sidekick --base-url http://localhost:11434/v1 --model qwen3-coder   # Ollama
```

## Other

Files can be ignored with `.gitignore`. It can also not read or list files outside of your
project.

The model must support tool calls and structured outputs (JSON schema).

If the project has an `AGENTS.md` or `CLAUDE.md`, sidekick reads it to learn your conventions.
