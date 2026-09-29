# Documentation guidance

Parent guidance in [AGENTS.md](../AGENTS.md) applies.

The [specification index](../specs/README.md) owns implemented behavior. These
guides explain architecture, setup, integration, and verification. Link to the
relevant spec instead of maintaining a second copy of a contract here.

- Keep module responsibilities and design rationale in `architecture.md`.
- Keep shell setup and client-development pointers in `shell-integration.md`.
- Keep portable bounded commands, test dependencies, fixture coverage, historical
  validation records, and troubleshooting in `vm-testing.md`. Test assertions
  may describe their scenarios; they do not replace the component contract.
- Preserve existing useful section anchors when replacing prose with spec links.
- Distinguish CPU tests, GPU/protocol fixtures, and physical input verification.
  Do not turn existing validation history into a claim of a new run.
- Keep machine-specific details in the ignored `.agents/` directory.

Markdown-only edits require text/source review, not builds or test execution,
unless the user explicitly requests it.
