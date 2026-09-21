import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const source = readFileSync(new URL("../src-tauri/src/process.rs", import.meta.url), "utf8");

test("window process handles are owned before image queries or early returns", () => {
  assert.match(source, /OpenProcess\([^;]+?\)\s*\}\s*\.map\(OwnedProcessHandle::new\)/s);
  assert.match(source, /impl Drop for OwnedProcessHandle[\s\S]*?CloseHandle\(self\.0\)/);
  assert.match(source, /collect_window_process\(ctx, pid, title_trim, process, query_process_path\)/);
  assert.match(source, /fn collect_window_process<H>[\s\S]*?process: H,[\s\S]*?query: impl FnOnce\(&H\)/);
});

test("process identity uses full paths while preserving the first PID and title", () => {
  assert.match(source, /use crate::runtime_policy::normalize_windows_path;/);
  assert.match(source, /let Some\(identity\) = normalize_windows_path\(&full_path\) else \{\s*return;\s*\}/);
  assert.match(source, /seen_paths\.insert\(identity\)/);
  assert.match(source, /path: full_path,/);
  assert.doesNotMatch(source, /seen_exes/);
  for (const regression of [
    "successful_and_duplicate_windows_close_every_handle",
    "failed_and_empty_queries_close_every_handle",
    "same_basename_different_paths_keep_distinct_process_rows",
    "runtime_equivalent_paths_collapse_without_changing_the_first_dto",
    "malformed_paths_are_rejected_and_every_handle_is_closed",
  ]) assert.ok(source.includes(`fn ${regression}(`), `missing deterministic regression: ${regression}`);
});
