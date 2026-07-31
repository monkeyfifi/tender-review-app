# Environment Setup And Model Test Status Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a visible local environment status and setup entry, and make the model-test stage complete during successful review.

**Architecture:** Reuse the existing word-format-checker resource and Python checks. Add two Tauri commands, extend the frontend API/type layer, then render a second status indicator and dialog in the existing top bar.

**Tech Stack:** Tauri v2, Rust, TypeScript, Vitest, Cargo tests.

## Global Constraints

- Do not add new dependencies.
- Do not silently install Python or packages.
- Preserve existing report files and reset-task behavior.
- Keep user-facing labels in Chinese.
- Keep the Windows batch file bundled through the existing Tauri resource config.

---

### Task 1: Backend Environment Commands

**Files:**
- Modify: `src-tauri/src/review/word_format_checker.rs`
- Create: `src-tauri/src/commands/environment.rs`
- Modify: `src-tauri/src/commands/mod.rs`
- Modify: `src-tauri/src/lib.rs`

**Interfaces:**
- Produces: `LocalEnvironmentStatus { state, message }`
- Produces: `technical_environment_status() -> LocalEnvironmentStatus`
- Produces: Tauri commands `get_local_environment_status` and `open_local_environment_setup`

- [ ] Write failing Rust tests for status and non-Windows setup-script behavior.
- [ ] Run targeted Cargo tests and confirm they fail.
- [ ] Expose reusable status detection from `word_format_checker.rs`.
- [ ] Add the environment command module and register commands in `lib.rs`.
- [ ] Run targeted Cargo tests and confirm they pass.

### Task 2: Model Test Stage

**Files:**
- Modify: `src-tauri/src/jobs/service.rs`

**Interfaces:**
- Consumes: existing `JobManifest::begin` and `JobManifest::complete`
- Produces: successful `run_review` statuses with `modelTest: complete`

- [ ] Write a failing assertion in the existing review-output test for `modelTest`.
- [ ] Run the targeted Cargo test and confirm it fails.
- [ ] Mark `ModelTest` running and complete before requirement extraction.
- [ ] Run the targeted Cargo test and confirm it passes.

### Task 3: Frontend Environment UI

**Files:**
- Modify: `src/types.ts`
- Modify: `src/api.ts`
- Modify: `src/app.ts`
- Modify: `src/styles.css`
- Modify: `tests/app.test.ts`

**Interfaces:**
- Consumes: `BackendApi.getLocalEnvironmentStatus()`
- Consumes: `BackendApi.openLocalEnvironmentSetup()`
- Produces: top-bar environment light and `环境设置` dialog

- [ ] Write failing Vitest coverage for the environment light and dialog actions.
- [ ] Run the targeted frontend test and confirm it fails.
- [ ] Add types and API invoke bindings.
- [ ] Add draft environment state, dialog rendering, recheck, and setup-script event handlers.
- [ ] Add minimal CSS for the second status group.
- [ ] Run the targeted frontend test and confirm it passes.

### Task 4: Verification

**Files:**
- No source changes expected.

- [ ] Run `npm test`.
- [ ] Run `npm run build`.
- [ ] Run `cargo fmt -- --check`.
- [ ] Run `cargo clippy -- -D warnings`.
- [ ] Run `cargo test`.
- [ ] Report remaining Windows manual checks.
