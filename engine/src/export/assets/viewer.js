(function () {
  "use strict";

  const pristineRoot =
    typeof document !== "undefined" && document.documentElement
      ? document.documentElement.cloneNode(true)
      : null;

  function safeJsonForScript(value) {
    const raw = typeof value === "string" ? value : JSON.stringify(value);
    return raw
      .replace(/</g, "\\u003c")
      .replace(/>/g, "\\u003e")
      .replace(/&/g, "\\u0026")
      .replace(/\u2028/g, "\\u2028")
      .replace(/\u2029/g, "\\u2029");
  }

  function buildAdjacency(edges) {
    const fwd = new Map();
    const rev = new Map();
    for (const e of edges || []) {
      if (!fwd.has(e.from)) fwd.set(e.from, []);
      if (!rev.has(e.to)) rev.set(e.to, []);
      fwd.get(e.from).push(e);
      rev.get(e.to).push(e);
    }
    return { fwd, rev };
  }

  function computeCausalCone(meta, nodeId) {
    const nodeIds = new Set();
    const edgeIds = new Set();
    if (!meta || !nodeId) return { nodeIds, edgeIds };
    const { fwd, rev } = buildAdjacency(meta.edges);

    const ancNodes = new Set([nodeId]);
    const ancQ = [nodeId];
    while (ancQ.length > 0) {
      const cur = ancQ.shift();
      for (const e of rev.get(cur) || []) {
        edgeIds.add(e.id);
        if (!ancNodes.has(e.from)) {
          ancNodes.add(e.from);
          ancQ.push(e.from);
        }
      }
    }

    const descNodes = new Set([nodeId]);
    const descQ = [nodeId];
    while (descQ.length > 0) {
      const cur = descQ.shift();
      for (const e of fwd.get(cur) || []) {
        edgeIds.add(e.id);
        if (!descNodes.has(e.to)) {
          descNodes.add(e.to);
          descQ.push(e.to);
        }
      }
    }

    for (const id of ancNodes) nodeIds.add(id);
    for (const id of descNodes) nodeIds.add(id);
    return { nodeIds, edgeIds };
  }

  function computeNullPaths(meta) {
    const keep = new Set();
    if (!meta || !meta.nodes) return keep;
    const { fwd, rev } = buildAdjacency(meta.edges);
    const fwdQ = [];
    const revQ = [];
    for (const n of meta.nodes) {
      if (n.absence_class === "Absent" || n.absence_class === "Nullable") {
        keep.add(n.id);
        fwdQ.push(n.id);
        revQ.push(n.id);
      }
    }
    const seenFwd = new Set(keep);
    while (fwdQ.length > 0) {
      const cur = fwdQ.shift();
      for (const e of fwd.get(cur) || []) {
        if (!seenFwd.has(e.to)) {
          seenFwd.add(e.to);
          keep.add(e.to);
          fwdQ.push(e.to);
        }
      }
    }
    const seenRev = new Set(keep);
    while (revQ.length > 0) {
      const cur = revQ.shift();
      for (const e of rev.get(cur) || []) {
        if (!seenRev.has(e.from)) {
          seenRev.add(e.from);
          keep.add(e.from);
          revQ.push(e.from);
        }
      }
    }
    return keep;
  }

  function evaluateFilters(meta, state) {
    const s = state || {};
    const anySemantic = Boolean(s.nullOnly || s.guards || s.unverified);
    const hasFile = s.fileIndex !== null && s.fileIndex !== undefined;
    const active = anySemantic || hasFile;
    const nodeIds = new Set();
    const edgeIds = new Set();
    if (!meta || !meta.nodes) return { active, nodeIds, edgeIds };

    if (!active) {
      for (const n of meta.nodes) nodeIds.add(n.id);
      for (const e of meta.edges || []) edgeIds.add(e.id);
      return { active: false, nodeIds, edgeIds };
    }

    let semanticSet;
    if (anySemantic) {
      semanticSet = new Set();
      if (s.nullOnly) {
        for (const id of computeNullPaths(meta)) semanticSet.add(id);
      }
      for (const n of meta.nodes) {
        if ((s.guards && n.is_guard) || (s.unverified && n.is_unverified)) {
          semanticSet.add(n.id);
        }
      }
    } else {
      semanticSet = new Set(meta.nodes.map((n) => n.id));
    }

    for (const n of meta.nodes) {
      if (semanticSet.has(n.id) && (!hasFile || n.file_index === s.fileIndex)) {
        nodeIds.add(n.id);
      }
    }
    for (const e of meta.edges || []) {
      if (nodeIds.has(e.from) && nodeIds.has(e.to)) edgeIds.add(e.id);
    }
    return { active: true, nodeIds, edgeIds };
  }

  function buildSnippetFreeHtml(rootNode) {
    if (!rootNode) return "";
    const clone = rootNode.cloneNode(true);
    const snipEl = clone.querySelector("#lineage-snippets");
    if (snipEl) snipEl.textContent = "{}";
    const metaEl = clone.querySelector("#lineage-meta");
    if (metaEl && metaEl.textContent) {
      const metaObj = JSON.parse(metaEl.textContent);
      metaObj.snippets_removed = true;
      metaEl.textContent = safeJsonForScript(metaObj);
    }
    const pill = clone.querySelector("#snippet-pill");
    if (pill) {
      pill.textContent = "Snippets removed";
      pill.className = "pill pill-off";
    }
    const saveBtn = clone.querySelector("#btn-save-no-snippets");
    if (saveBtn && saveBtn.parentNode) saveBtn.parentNode.removeChild(saveBtn);
    return "<!DOCTYPE html>\n" + clone.outerHTML;
  }

  if (typeof module !== "undefined" && module.exports) {
    module.exports = {
      safeJsonForScript,
      computeCausalCone,
      computeNullPaths,
      evaluateFilters,
      buildSnippetFreeHtml,
    };
  }

  if (typeof document === "undefined") return;
  const metaScript = document.getElementById("lineage-meta");
  const snipScript = document.getElementById("lineage-snippets");
  if (!metaScript || !snipScript) return;

  const meta = JSON.parse(metaScript.textContent || "{}");
  const snippets = JSON.parse(snipScript.textContent || "{}");
  const nodeMap = new Map((meta.nodes || []).map((n) => [n.id, n]));

  const svg = document.getElementById("graph-svg");
  const viewport = document.getElementById("viewport");
  const activeOverlay = document.getElementById("active-overlay");
  const canvasWrap = document.getElementById("canvas-wrap");
  const inspector = document.getElementById("inspector");

  const filterState = {
    nullOnly: false,
    guards: false,
    unverified: false,
    fileIndex: null,
    selectedId: null,
  };
  let tx = 0;
  let ty = 0;
  let scale = 1;

  function applyTransform() {
    viewport.setAttribute("transform", `translate(${tx} ${ty}) scale(${scale})`);
  }

  function fitBox(minX, minY, w, h) {
    const rect = canvasWrap.getBoundingClientRect();
    if (!rect || rect.width <= 0 || rect.height <= 0 || w <= 0 || h <= 0) return;
    const pad = 28;
    scale = Math.min((rect.width - pad * 2) / w, (rect.height - pad * 2) / h, 1.5);
    scale = Math.max(scale, 0.15);
    tx = (rect.width - w * scale) / 2 - minX * scale;
    ty = (rect.height - h * scale) / 2 - minY * scale;
    applyTransform();
  }

  function fitView() {
    fitBox(0, 0, meta.view_width || 640, meta.view_height || 240);
  }

  function centerFocus() {
    const rid = (meta.root_ids && meta.root_ids[0]) || (meta.nodes && meta.nodes[0] && meta.nodes[0].id);
    const node = rid ? nodeMap.get(rid) : null;
    const rect = canvasWrap.getBoundingClientRect();
    if (!node || !rect || rect.width <= 0 || rect.height <= 0) return fitView();
    scale = 1;
    tx = rect.width / 2 - (node.bounds.x + node.bounds.w / 2);
    ty = rect.height / 2 - (node.bounds.y + node.bounds.h / 2);
    applyTransform();
  }

  function fitActive() {
    const activeEls = Array.from(document.querySelectorAll(".node-group.active-node"));
    if (activeEls.length === 0) return fitView();
    let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
    for (const el of activeEls) {
      const n = nodeMap.get(el.getAttribute("data-node-id"));
      if (!n) continue;
      minX = Math.min(minX, n.bounds.x);
      minY = Math.min(minY, n.bounds.y);
      maxX = Math.max(maxX, n.bounds.x + n.bounds.w);
      maxY = Math.max(maxY, n.bounds.y + n.bounds.h);
    }
    if (!isFinite(minX)) return fitView();
    fitBox(minX, minY, maxX - minX, maxY - minY);
  }

  function addSection(parent, title, text, isCode) {
    const sec = document.createElement("div");
    sec.className = "inspector-section";
    const lbl = document.createElement("div");
    lbl.className = "inspector-label";
    lbl.textContent = title;
    const val = document.createElement("div");
    val.className = isCode ? "inspector-code" : "";
    val.textContent = text;
    sec.appendChild(lbl);
    sec.appendChild(val);
    parent.appendChild(sec);
  }

  function showCopyToast(parent, locText) {
    const existing = parent.querySelector(".copy-toast");
    if (existing) existing.remove();
    const box = document.createElement("div");
    box.className = "copy-toast";
    const msg = document.createElement("span");
    msg.textContent = "Clipboard blocked on file:// — press Ctrl+C / Cmd+C";
    const inp = document.createElement("input");
    inp.readOnly = true;
    inp.value = locText;
    box.appendChild(msg);
    box.appendChild(inp);
    parent.appendChild(box);
    inp.focus({ preventScroll: true });
    inp.select();
  }

  function copyPathLine(locText, btn, container) {
    const markCopied = () => {
      const prev = btn.textContent;
      btn.textContent = "Copied!";
      setTimeout(() => { btn.textContent = prev; }, 1500);
    };
    if (window.isSecureContext && navigator.clipboard && navigator.clipboard.writeText) {
      navigator.clipboard.writeText(locText).then(markCopied, () => showCopyToast(container, locText));
      return;
    }
    let ok = false;
    try {
      const ta = document.createElement("textarea");
      ta.readOnly = true;
      ta.value = locText;
      ta.style.position = "fixed";
      ta.style.top = "0";
      ta.style.left = "0";
      ta.style.opacity = "0";
      document.body.appendChild(ta);
      ta.focus({ preventScroll: true });
      ta.select();
      ok = document.execCommand("copy");
      document.body.removeChild(ta);
    } catch (_) {
      ok = false;
    }
    if (ok) markCopied();
    else showCopyToast(container, locText);
  }

  function renderInspector(node) {
    inspector.replaceChildren();
    if (!node) {
      inspector.classList.remove("open");
      return;
    }
    inspector.classList.add("open");
    const hdr = document.createElement("div");
    hdr.className = "inspector-header";
    const title = document.createElement("div");
    title.className = "inspector-title";
    title.textContent = node.label || node.symbol || node.id;
    const closeBtn = document.createElement("button");
    closeBtn.type = "button";
    closeBtn.textContent = "Close";
    closeBtn.addEventListener("click", () => {
      filterState.selectedId = null;
      refreshView();
    });
    hdr.appendChild(title);
    hdr.appendChild(closeBtn);
    inspector.appendChild(hdr);

    const locText = `${node.norm_path}:${node.line_1based}`;
    const copyRow = document.createElement("div");
    copyRow.className = "copy-row";
    const copyBtn = document.createElement("button");
    copyBtn.type = "button";
    copyBtn.textContent = "Copy path:line";
    copyBtn.addEventListener("click", () => copyPathLine(locText, copyBtn, inspector));
    copyRow.appendChild(copyBtn);
    inspector.appendChild(copyRow);

    addSection(inspector, "Location", `${locText}:${node.col_1based}`, true);
    addSection(inspector, "Absence & Provenance", `${node.absence_class} · ${node.provenance || "static"}`, false);
    if (node.unknown_reasons && node.unknown_reasons.length > 0) {
      addSection(inspector, "Unknown Reasons", node.unknown_reasons.join(", "), false);
    }
    if (node.guard_text) addSection(inspector, "Guard", node.guard_text, true);
    if (node.risk_messages && node.risk_messages.length > 0) {
      addSection(inspector, "Local Risks", node.risk_messages.join("\n"), true);
    }
    if (snippets[node.id]) {
      addSection(inspector, "Snippet (trimmed)", snippets[node.id], true);
    } else {
      addSection(inspector, "Snippet", "Omitted", false);
    }

    const hops = (meta.edges || []).filter((e) => e.from === node.id || e.to === node.id);
    if (hops.length > 0) {
      const sec = document.createElement("div");
      sec.className = "inspector-section";
      const lbl = document.createElement("div");
      lbl.className = "inspector-label";
      lbl.textContent = "Direct Causal Links";
      const list = document.createElement("div");
      list.className = "hop-list";
      for (const e of hops) {
        const isOut = e.from === node.id;
        const targetId = isOut ? e.to : e.from;
        const targetNode = nodeMap.get(targetId);
        const btn = document.createElement("button");
        btn.type = "button";
        btn.className = "hop-btn";
        const left = document.createElement("span");
        left.textContent = (targetNode && (targetNode.label || targetNode.symbol)) || targetId;
        const right = document.createElement("span");
        right.className = "hop-tag";
        const tag = e.label || (e.is_back_edge ? "cycle" : "flow");
        right.textContent = `${isOut ? "→ out" : "← in"} · ${tag}`;
        btn.appendChild(left);
        btn.appendChild(right);
        btn.addEventListener("click", () => {
          filterState.selectedId = targetId;
          refreshView();
        });
        list.appendChild(btn);
      }
      sec.appendChild(lbl);
      sec.appendChild(list);
      inspector.appendChild(sec);
    }
  }

  function refreshView() {
    const fRes = evaluateFilters(meta, filterState);
    const cone = filterState.selectedId ? computeCausalCone(meta, filterState.selectedId) : null;
    const dim = fRes.active || Boolean(cone);
    svg.classList.toggle("graph-dimmed", dim);

    const activeNodes = cone ? new Set([...fRes.nodeIds].filter((id) => cone.nodeIds.has(id))) : fRes.nodeIds;
    const activeEdges = cone ? new Set([...fRes.edgeIds].filter((id) => cone.edgeIds.has(id))) : fRes.edgeIds;

    for (const el of document.querySelectorAll("#nodes-layer .node-group")) {
      const id = el.getAttribute("data-node-id");
      el.classList.toggle("active-node", !dim || activeNodes.has(id));
      el.classList.toggle("selected", id === filterState.selectedId);
    }
    activeOverlay.replaceChildren();
    for (const el of document.querySelectorAll("#edges-layer .edge-group")) {
      const id = el.getAttribute("data-edge-id");
      const isAct = !dim || activeEdges.has(id);
      el.classList.toggle("active-edge", isAct);
      if (dim && isAct) activeOverlay.appendChild(el.cloneNode(true));
    }

    document.getElementById("btn-null-only").classList.toggle("active", filterState.nullOnly);
    document.getElementById("btn-guards").classList.toggle("active", filterState.guards);
    document.getElementById("btn-unverified").classList.toggle("active", filterState.unverified);
    for (const btn of document.querySelectorAll(".legend-item")) {
      btn.classList.toggle("active", Number(btn.getAttribute("data-file-index")) === filterState.fileIndex);
    }
    renderInspector(filterState.selectedId ? nodeMap.get(filterState.selectedId) : null);
  }

  const nullCount = computeNullPaths(meta).size;
  const guardCount = (meta.nodes || []).filter((n) => n.is_guard).length;
  const unvCount = (meta.nodes || []).filter((n) => n.is_unverified).length;
  const setupToggle = (id, label, count, key) => {
    const btn = document.getElementById(id);
    if (!btn) return;
    const lblEl = btn.querySelector(".filter-label") || btn;
    lblEl.textContent = `${label} (${count})`;
    btn.disabled = count === 0;
    btn.addEventListener("click", () => {
      filterState[key] = !filterState[key];
      refreshView();
    });
  };
  setupToggle("btn-null-only", "Null-only", nullCount, "nullOnly");
  setupToggle("btn-guards", "Guards", guardCount, "guards");
  setupToggle("btn-unverified", "Unverified", unvCount, "unverified");

  for (const btn of document.querySelectorAll(".legend-item")) {
    btn.addEventListener("click", () => {
      const idx = Number(btn.getAttribute("data-file-index"));
      filterState.fileIndex = filterState.fileIndex === idx ? null : idx;
      refreshView();
    });
  }

  for (const el of document.querySelectorAll("#nodes-layer .node-group")) {
    el.addEventListener("click", (ev) => {
      ev.stopPropagation();
      const id = el.getAttribute("data-node-id");
      filterState.selectedId = filterState.selectedId === id ? null : id;
      refreshView();
    });
  }

  let dragStart = null;
  let moved = false;
  canvasWrap.addEventListener("pointerdown", (ev) => {
    if (ev.button !== 0 || ev.target.closest(".node-group") || ev.target.closest(".canvas-controls")) return;
    dragStart = { x: ev.clientX - tx, y: ev.clientY - ty };
    moved = false;
    canvasWrap.classList.add("dragging");
  });
  window.addEventListener("pointermove", (ev) => {
    if (!dragStart) return;
    moved = true;
    tx = ev.clientX - dragStart.x;
    ty = ev.clientY - dragStart.y;
    applyTransform();
  });
  window.addEventListener("pointerup", () => {
    if (dragStart && !moved && filterState.selectedId) {
      filterState.selectedId = null;
      refreshView();
    }
    dragStart = null;
    canvasWrap.classList.remove("dragging");
  });
  canvasWrap.addEventListener("wheel", (ev) => {
    ev.preventDefault();
    const rect = canvasWrap.getBoundingClientRect();
    const cx = ev.clientX - rect.left;
    const cy = ev.clientY - rect.top;
    const factor = ev.deltaY < 0 ? 1.12 : 0.89;
    const next = Math.min(Math.max(scale * factor, 0.15), 4);
    tx = cx - ((cx - tx) * next) / scale;
    ty = cy - ((cy - ty) * next) / scale;
    scale = next;
    applyTransform();
  }, { passive: false });

  document.getElementById("btn-zoom-in").addEventListener("click", () => { scale = Math.min(scale * 1.2, 4); applyTransform(); });
  document.getElementById("btn-zoom-out").addEventListener("click", () => { scale = Math.max(scale / 1.2, 0.15); applyTransform(); });
  document.getElementById("btn-fit-view").addEventListener("click", fitView);
  document.getElementById("btn-fit-active").addEventListener("click", fitActive);
  document.getElementById("btn-center-focus").addEventListener("click", centerFocus);
  document.getElementById("btn-reset").addEventListener("click", () => {
    filterState.nullOnly = false;
    filterState.guards = false;
    filterState.unverified = false;
    filterState.fileIndex = null;
    filterState.selectedId = null;
    refreshView();
    fitView();
  });
  document.getElementById("btn-legend-toggle").addEventListener("click", () => {
    document.getElementById("legend-panel").classList.toggle("collapsed");
  });
  document.getElementById("btn-theme").addEventListener("click", () => {
    const cur = document.documentElement.getAttribute("data-theme");
    const darkMedia = window.matchMedia && window.matchMedia("(prefers-color-scheme: dark)").matches;
    const isDark = cur === "dark" || (!cur && darkMedia);
    document.documentElement.setAttribute("data-theme", isDark ? "light" : "dark");
  });

  const saveBtn = document.getElementById("btn-save-no-snippets");
  if (saveBtn) {
    if (Object.keys(snippets).length === 0) {
      saveBtn.classList.add("hidden");
    } else {
      saveBtn.addEventListener("click", () => {
        const html = buildSnippetFreeHtml(pristineRoot);
        const blob = new Blob([html], { type: "text/html;charset=utf-8" });
        const url = URL.createObjectURL(blob);
        const a = document.createElement("a");
        a.href = url;
        a.download = "lineage-no-snippets.html";
        document.body.appendChild(a);
        a.click();
        document.body.removeChild(a);
        setTimeout(() => URL.revokeObjectURL(url), 1000);
      });
    }
  }

  window.addEventListener("keydown", (ev) => {
    if (ev.key === "Escape" && filterState.selectedId) {
      filterState.selectedId = null;
      refreshView();
    } else if (ev.key === "f") {
      fitActive();
    } else if (ev.key === "r") {
      document.getElementById("btn-reset").click();
    }
  });

  if ((meta.nodes || []).length <= 25) fitView();
  else centerFocus();
})();
