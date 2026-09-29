## What and why

<!-- What does this change, and why? Link the issue if there is one. -->

## How it was tested

<!-- The tests you added, and the commands you ran. -->

- [ ] `cargo xtask check`
- [ ] `cargo xtask test linux`
- [ ] Other suites that apply (`test wasm`, `sanitize`, `miri`), or why they do not

## Checklist

- [ ] The change does one thing, and the commit subjects follow `Area: summary`
- [ ] New behaviour has tests that fail without the change
- [ ] Every new `unsafe` block is in `raw` and has a specific `SAFETY:` comment
- [ ] Tests that use threads are gated for WebAssembly
- [ ] `CHANGELOG.md` and the docs are updated for user-visible changes

## AI assistance

<!-- If a substantial part of the code, tests or description was AI-generated, say so
here, and confirm that you have read, run and understood it. Delete this section if it
does not apply. -->
