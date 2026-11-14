# Bump Guidance

1. Do not edit native manifest versions directly.
2. Do not run `omv bump` or `omv sync` locally; the release pipeline assigns versions.
3. Use `omv current --json`, `omv plan --json`, and `omv sync --check --json` for reads.
