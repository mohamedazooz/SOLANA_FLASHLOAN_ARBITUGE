# Model Assignment Snapshot — 2026-11-10

## Recommended low-cost paid

- `z-ai/glm-5.3-flash` — OpenRouter listing: $0.0335 / $0.083 per 1M input/output; 1M context; tool calls and structured outputs; coding index shown as 71.5.
- `deepseek/deepseek-v4.1-flash-1017` — OpenRouter listing: $0.006 / $0.50 per 1M input/output; 1M context; marketed for coding, terminal, computer-use and long-horizon agents. Use this for Rust/execution work, and use a different model for independent PnL review. Pricing/provider availability can change; verify in the live model picker before use.
- `stepfun/step-5-preview` — $1.00 / $2.70 per 1M input/output; 1M context; reserve for hard tasks only.

## Free options

- `thinkingmachines/inkling-2-small:free` — free, 1M context and tool calling; released 2026-11-09, so early-access/reliability risk remains.
- `nvidia/nemotron-3.5-super-vl:free` — free, 1M context; strong availability in a 10-day, 282-request third-party test.
- `nvidia/nemotron-3-ultra-550b-a55b:free` — free, 1M context; 7/7 code checks in the cited test but slower median generation than Super VL.
- `cohere/north-mini-code:free` — free, 256K context; good tool-call reliability in that test; coding subset 5/7.

The test was a small endpoint reliability sample, not a universal benchmark. Never treat a model score or agent answer as proof of financial correctness.
