import assert from "node:assert/strict";
import { createRequire } from "node:module";
import test from "node:test";

const require = createRequire(import.meta.url);
const viewer = require("../src/export/assets/viewer.js");

test("safeJsonForScript escapes script-breakout and line separator characters", () => {
  const payload = {
    sym: "</script><img onerror=alert(1)><!-- & \u2028 \u2029",
  };
  const encoded = viewer.safeJsonForScript(payload);
  assert.ok(!encoded.includes("</script>"));
  assert.ok(!encoded.includes("<!--"));
  assert.ok(!encoded.includes("&"));
  assert.ok(!encoded.includes("\u2028"));
  assert.ok(!encoded.includes("\u2029"));
  assert.ok(encoded.includes("\\u003c/script\\u003e"));
  assert.ok(encoded.includes("\\u0026"));
  assert.ok(encoded.includes("\\u2028"));
  assert.ok(encoded.includes("\\u2029"));
  assert.deepEqual(JSON.parse(encoded), payload);
});

test("computeCausalCone is transitive and cycle-safe", () => {
  const meta = {
    nodes: [{ id: "N0" }, { id: "N1" }, { id: "N2" }, { id: "N3" }],
    edges: [
      { id: "E0", from: "N0", to: "N1" },
      { id: "E1", from: "N1", to: "N2" },
      { id: "E2", from: "N2", to: "N0" },
      { id: "E3", from: "N3", to: "N2" },
    ],
  };
  const cone = viewer.computeCausalCone(meta, "N1");
  assert.deepEqual([...cone.nodeIds].sort(), ["N0", "N1", "N2", "N3"]);
  assert.ok(cone.edgeIds.has("E0"));
  assert.ok(cone.edgeIds.has("E1"));
  assert.ok(cone.edgeIds.has("E3"));
});

test("computeNullPaths and evaluateFilters compose OR-union and file intersection", () => {
  const meta = {
    nodes: [
      { id: "N0", file_index: 0, absence_class: "Nullable", is_guard: false, is_unverified: false },
      { id: "N1", file_index: 1, absence_class: "NonNull", is_guard: false, is_unverified: false },
      { id: "N2", file_index: 1, absence_class: "NonNull", is_guard: true, is_unverified: false },
      { id: "N3", file_index: 0, absence_class: "NonNull", is_guard: false, is_unverified: true },
    ],
    edges: [{ id: "E0", from: "N0", to: "N1" }],
  };

  const nullPaths = viewer.computeNullPaths(meta);
  assert.deepEqual([...nullPaths].sort(), ["N0", "N1"]);

  const orFiltered = viewer.evaluateFilters(meta, {
    nullOnly: true,
    guards: true,
    unverified: false,
    fileIndex: null,
  });
  assert.equal(orFiltered.active, true);
  assert.deepEqual([...orFiltered.nodeIds].sort(), ["N0", "N1", "N2"]);

  const fileIntersected = viewer.evaluateFilters(meta, {
    nullOnly: true,
    guards: true,
    unverified: false,
    fileIndex: 1,
  });
  assert.deepEqual([...fileIntersected.nodeIds].sort(), ["N1", "N2"]);
});

test("buildSnippetFreeHtml clears snippets, escapes meta, and is idempotent", () => {
  const evilMeta = {
    viewer_schema_version: 1,
    nodes: [{ id: "N0", symbol: "</script><img onerror=alert(1)>" }],
    snippets_removed: false,
  };

  const makeMockRoot = () => {
    const snippetsEl = { textContent: '{"N0":"const secret = 1;"}' };
    const metaEl = { textContent: viewer.safeJsonForScript(evilMeta) };
    const pillEl = { textContent: "Snippets: ON (local)", className: "pill pill-on" };
    const saveBtn = {
      removed: false,
      parentNode: {
        removeChild() {
          saveBtn.removed = true;
        },
      },
    };
    const root = {
      cloneNode() {
        const cloned = makeMockRoot();
        return cloned;
      },
      querySelector(sel) {
        if (sel === "#lineage-snippets") return snippetsEl;
        if (sel === "#lineage-meta") return metaEl;
        if (sel === "#snippet-pill") return pillEl;
        if (sel === "#btn-save-no-snippets") return saveBtn;
        return null;
      },
      get outerHTML() {
        return (
          `<html><span id="snippet-pill" class="${pillEl.className}">${pillEl.textContent}</span>` +
          `<script id="lineage-meta">${metaEl.textContent}</script>` +
          `<script id="lineage-snippets">${snippetsEl.textContent}</script></html>`
        );
      },
    };
    return root;
  };

  const html = viewer.buildSnippetFreeHtml(makeMockRoot());
  assert.ok(html.startsWith("<!DOCTYPE html>\n"));
  assert.ok(!html.includes("const secret = 1;"));
  assert.ok(!html.includes("</script><img"));
  assert.ok(html.includes("\\u003c/script\\u003e"));
  assert.ok(html.includes("Snippets removed"));
  assert.ok(html.includes('"snippets_removed":true'));
});
