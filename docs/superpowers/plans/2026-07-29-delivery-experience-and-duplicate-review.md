# Delivery Experience and Duplicate Review Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Persist saved API keys, clarify task progress/reset behavior, hide implementation names, and locate continuous technical-file duplicates.

**Architecture:** Keep keys in the OS credential store, reset only the TypeScript draft, and replace whole-document term sets with block-level exact fragment matching.

**Tech Stack:** Rust, Tauri, `keyring`, TypeScript, Vitest, Markdown, normalized-document anchors.

## Global Constraints

- API Key is stored only in the OS credential manager, never in JSON configuration, task manifests, or Markdown.
- Empty API Key preserves a saved key; only `clear_model_key` removes it.
- Reset preserves completed task directories and reports.
- User-facing Markdown and previews cannot include `tender-review-skill` or `word-format-checker`.
- Comparison considers linked technical blind-bid DOCX files only and emits exact continuous fragments at least 12 normalized characters long.
- No model request or new dependency is added for comparison.

---

### Task 1: Persist API keys by default

**Files:**
- Modify: `src-tauri/src/config/model.rs`
- Modify: `src-tauri/src/commands/config.rs`
- Modify: `src/types.ts`
- Modify: `src/app.ts`
- Test: `src-tauri/src/commands/config.rs`
- Test: `tests/app.test.ts`

**Interfaces:** `SaveModelSettingsInput` contains only `base_url`, `model`, `timeout_seconds`, and optional `api_key`. `ModelConfigurationState::effective_key()` loads only the credential store.

- [ ] **Step 1: Write the failing tests**

```rust
assert_eq!(state.effective_key().unwrap().as_deref(), Some("saved-key"));
state.clear_key().unwrap();
assert_eq!(state.effective_key().unwrap(), None);
```

```ts
expect(api.saveModelSettings).toHaveBeenCalledWith(expect.objectContaining({ apiKey: "saved-key" }));
expect(document.querySelector('[name="rememberKey"]')).toBeNull();
```

- [ ] **Step 2: Verify failure**

Run: `npm test -- --run tests/app.test.ts` and `PATH="/Users/zhaoyun/.cargo/bin:$PATH" cargo test commands::config::tests`

Expected: the old UI exposes `rememberKey`, and the old backend still has a process-only path.

- [ ] **Step 3: Implement minimal persistent save**

```rust
match input.api_key.map(|key| key.trim().to_owned()).filter(|key| !key.is_empty()) {
    Some(api_key) => self.credential_store.save_key(&api_key)?,
    None => {}
}
```

Remove `remember_key`, `clear_remembered_key`, and `transient_api_key`. Retain `clear_key()` as the sole deletion path. Remove the checkbox and process-only explanatory text from the API dialog.

- [ ] **Step 4: Verify and commit**

Run: `npm test -- --run tests/app.test.ts` and `PATH="/Users/zhaoyun/.cargo/bin:$PATH" cargo test commands::config::tests`

Commit: `git add src-tauri/src/config/model.rs src-tauri/src/commands/config.rs src/types.ts src/app.ts tests/app.test.ts && git commit -m "fix: persist saved model keys by default"`

### Task 2: Make progress and reset behavior explicit

**Files:**
- Modify: `src/app.ts`
- Modify: `src/styles.css`
- Test: `tests/app.test.ts`

**Interfaces:** `workflowSteps(draft): string` renders completed/current states. `resetTask(): void` clears only draft state and never calls `BackendApi.clearJob`.

- [ ] **Step 1: Write the failing test**

```ts
expect(document.querySelector('.steps .active')?.textContent).toContain("批量审核");
document.querySelector<HTMLButtonElement>('[data-action="reset-task"]')?.click();
expect(app.getDraft().phase).toBe("editing");
expect(api.clearJob).not.toHaveBeenCalled();
```

- [ ] **Step 2: Verify failure**

Run: `npm test -- --run tests/app.test.ts`

Expected: current step always remains "添加文件" and no reset button exists.

- [ ] **Step 3: Implement phase-driven layout**

```ts
const resetTask = () => update({
  tender: null, bids: [], errors: [], preparedFiles: [], jobId: null,
  reviewStatus: null, reportPath: null, reportMarkdown: null,
  selectedReportPath: null, phase: "editing",
});
```

Render progress/result summaries before file cards for `preflight`, `ready`, `reviewing`, and `completed`. Derive completed/current step CSS from phase and show `重置任务` only for completed and failed tasks.

- [ ] **Step 4: Verify and commit**

Run: `npm test -- --run tests/app.test.ts`

Commit: `git add src/app.ts src/styles.css tests/app.test.ts && git commit -m "feat: clarify workflow progress and reset tasks"`

### Task 3: Use business labels in audit results

**Files:**
- Modify: `src-tauri/src/jobs/service.rs`
- Modify: `src-tauri/src/review/word_format_checker.rs`
- Test: `src-tauri/src/jobs/service.rs`
- Test: `src-tauri/src/review/word_format_checker.rs`

**Interfaces:** Business Markdown uses `商务审核说明`; technical Markdown uses `技术暗标格式检查`.

- [ ] **Step 1: Write the failing test**

```rust
assert!(business.markdown.contains("商务审核说明"));
assert!(!business.markdown.contains("tender-review-skill"));
assert!(blind_bid.markdown.contains("技术暗标格式检查"));
assert!(!blind_bid.markdown.contains("word-format-checker"));
```

- [ ] **Step 2: Verify failure**

Run: `PATH="/Users/zhaoyun/.cargo/bin:$PATH" cargo test review_outputs_separate_markdown_files_and_compares_only_blind_bids`

Expected: the current generated Markdown exposes both implementation names.

- [ ] **Step 3: Replace user-facing strings only**

Use `## 商务审核说明`, `本结果按商务审核清单组织，需人工复核。`, `## 技术暗标格式检查`, and `技术暗标格式检查完成，需人工复核`. Preserve checker module names, resource paths, and diagnostic errors.

- [ ] **Step 4: Verify and commit**

Run: `PATH="/Users/zhaoyun/.cargo/bin:$PATH" cargo test review_outputs_separate_markdown_files_and_compares_only_blind_bids` and `PATH="/Users/zhaoyun/.cargo/bin:$PATH" cargo test review::word_format_checker::tests`

Commit: `git add src-tauri/src/jobs/service.rs src-tauri/src/review/word_format_checker.rs && git commit -m "fix: use business labels in audit results"`

### Task 4: Locate continuous duplicate fragments

**Files:**
- Modify: `src-tauri/src/review/similarity.rs`
- Modify: `src-tauri/src/jobs/service.rs`
- Modify: `src-tauri/src/reports.rs`
- Test: `src-tauri/src/review/similarity.rs`
- Test: `src-tauri/src/jobs/service.rs`
- Test: `src-tauri/src/reports.rs`

**Interfaces:**

```rust
pub struct DuplicateFragment { pub text: String, pub left_location: String, pub right_location: String }
pub struct DuplicatePair { pub left_bid: usize, pub right_bid: usize, pub fragments: Vec<DuplicateFragment> }
pub fn compare_blind_documents(documents: &[(usize, NormalizedDocument)]) -> Result<Vec<DuplicatePair>, AppError>;
```

- [ ] **Step 1: Write failing anchored-block tests**

```rust
let pairs = compare_blind_documents(&[
    (1, document("投标文件1行8", "项目实施组织与质量保障措施。")),
    (2, document("投标文件2行3", "项目实施组织与质量保障措施。其他内容")),
]).unwrap();
assert!(pairs[0].fragments[0].text.contains("项目实施组织与质量保障措施"));
assert!(pairs[0].fragments[0].left_location.contains("投标文件1行8"));
assert!(compare_blind_documents(&[(1, document("行1", "技术方案甲")), (2, document("行1", "技术方案乙"))]).unwrap().is_empty());
```

- [ ] **Step 2: Verify failure**

Run: `PATH="/Users/zhaoyun/.cargo/bin:$PATH" cargo test review::similarity::tests`

Expected: old comparison returns common two-character terms instead of anchored fragments.

- [ ] **Step 3: Implement exact window matching**

```rust
const MIN_FRAGMENT_CHARS: usize = 12;
fn normalized_characters(text: &str) -> Vec<char> {
    text.nfc().collect::<String>().to_lowercase().chars()
        .filter(|character| !character.is_whitespace()).collect()
}
```

Index every left-block 12-character window. For a matching right-block window, extend backwards and forwards to the maximal exact fragment. Drop short fragments, deduplicate identical fragment text per file pair, record both `"{line_label} {structure_path}"` anchors, sort by fragment length, and omit pairs without fragments.

- [ ] **Step 4: Update workflow and Markdown**

Replace the call with `compare_blind_documents(&blind_documents)?`. Render a table of `重复片段`, `技术文件 A 位置`, `技术文件 B 位置`, and `人工复核提示`. If no pairs remain, output `未发现需重点复核的连续重复内容`. Update the legacy aggregate renderer in `reports.rs` for the new `fragments` field.

- [ ] **Step 5: Verify and commit**

Run: `PATH="/Users/zhaoyun/.cargo/bin:$PATH" cargo test review::similarity::tests`; `PATH="/Users/zhaoyun/.cargo/bin:$PATH" cargo test review_outputs_separate_markdown_files_and_compares_only_blind_bids`; `PATH="/Users/zhaoyun/.cargo/bin:$PATH" cargo test reports::tests`

Commit: `git add src-tauri/src/review/similarity.rs src-tauri/src/jobs/service.rs src-tauri/src/reports.rs && git commit -m "feat: locate continuous technical file duplicates"`

### Task 5: Full verification

**Files:** Modify only when a verification failure exposes a defect in Tasks 1-4.

- [ ] **Step 1: Verify frontend**

Run: `npm test && npm run build`

- [ ] **Step 2: Verify Rust**

Run: `cd src-tauri && PATH="/Users/zhaoyun/.cargo/bin:$PATH" cargo fmt -- --check && PATH="/Users/zhaoyun/.cargo/bin:$PATH" cargo clippy -- -D warnings && PATH="/Users/zhaoyun/.cargo/bin:$PATH" cargo test`

- [ ] **Step 3: Inspect final working tree**

Run: `git diff --check; git status --short`

Expected: all checks pass, no whitespace errors, and no unrelated change is reverted.
