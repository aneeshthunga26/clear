# Specification guidance

Parent guidance in [AGENTS.md](../AGENTS.md) applies.

`specs/` is the canonical reference for implemented Clear behavior. Read the
relevant specification before changing a component, and update it in the same
change as the implementation. Add a component specification and an index entry
when no existing document owns the behavior.

- Describe observable behavior, invariants, defaults, validation, lifecycle,
  failure handling, resource bounds, and current limitations as applicable.
- Use MUST, MUST NOT, and MAY for requirements as defined in [README.md](README.md).
- Ground requirements in code and tests. Link to their repository paths and name
  useful test cases; do not claim that reading a test establishes a passing run.
- Keep one owner for each contract. Link to other specs instead of duplicating
  their rules. Keep setup, tutorials, architecture rationale, commands for running
  tests, and historical validation records in `docs/` or the relevant guide.
- Describe implemented behavior separately from future work. Do not silently
  turn a proposal into a supported feature, or broaden a guarantee beyond the code.
- Keep local environment details out of tracked files.

For Markdown-only edits, review the text, relative links, and code/test references.
Builds, test suites, formatters, and compositor runs are unnecessary unless the
user specifically requests them.
