use std::collections::HashMap;

use tree_sitter::{Node, Parser};

use crate::adapters::python::stdlib::lookup_python_stdlib;
use crate::adapters::python::stub_extract::classify_py_type_text;
use crate::ir::guards::GuardFact;
use crate::ir::{
    AbsenceValue, BasicBlock, BlockId, CallTarget, DangerSink, DangerSinkKind, ExitEdge,
    FileSkeleton, FnSummary, FunctionIr, FunctionSkeleton, ImportSpec, SsiInst, SsiOp,
    ValueId, STATE_NULL, STATE_PRESENT,
};
use crate::protocol::schema::{CanonicalRange, UnknownReason};
use crate::text::PositionMapper;

pub fn parse_py_skeleton(uri: &str, source: &str) -> FileSkeleton {
    let content_hash = xxhash_rust::xxh3::xxh3_64(source.as_bytes());
    let mut parser = Parser::new();
    let lang = tree_sitter_python::LANGUAGE.into();
    if parser.set_language(&lang).is_err() {
        return empty_skeleton(uri, content_hash);
    }
    let Some(tree) = parser.parse(source, None) else {
        return empty_skeleton(uri, content_hash);
    };

    let mapper = PositionMapper::new(source);
    let src = source.as_bytes();
    let root = tree.root_node();
    let mut imports = Vec::new();
    let exports = Vec::new();
    let mut functions = Vec::new();
    let mut parse_errors = Vec::new();

    collect_parse_errors(root, &mapper, &mut parse_errors);

    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        match child.kind() {
            "import_statement" => extract_import_stmt(child, src, &mapper, &mut imports),
            "import_from_statement" => extract_import_from_stmt(child, src, &mapper, &mut imports),
            "function_definition" => {
                if let Some(fskel) = extract_fn_skeleton(child, src, &mapper, None) {
                    functions.push(fskel);
                }
            }
            "decorated_definition" => {
                if let Some(def) = child.child_by_field_name("definition") {
                    match def.kind() {
                        "function_definition" => {
                            if let Some(fskel) = extract_fn_skeleton(def, src, &mapper, None) {
                                functions.push(fskel);
                            }
                        }
                        "class_definition" => {
                            extract_class_methods(def, src, &mapper, &mut functions);
                        }
                        _ => {}
                    }
                }
            }
            "class_definition" => {
                extract_class_methods(child, src, &mapper, &mut functions);
            }
            _ => {}
        }
    }

    functions.push(FunctionSkeleton {
        symbol_id: "<module>".to_string(),
        name: "<module>".to_string(),
        range: mapper.byte_range_to_canonical(0, source.len()),
        start_byte: 0,
        end_byte: source.len(),
        content_hash,
        is_exported: false,
    });

    FileSkeleton {
        uri: uri.to_string(),
        content_hash,
        imports,
        exports,
        functions,
        parse_errors,
    }
}

fn empty_skeleton(uri: &str, content_hash: u64) -> FileSkeleton {
    FileSkeleton {
        uri: uri.to_string(),
        content_hash,
        imports: Vec::new(),
        exports: Vec::new(),
        functions: Vec::new(),
        parse_errors: Vec::new(),
    }
}

fn collect_parse_errors(node: Node<'_>, mapper: &PositionMapper, out: &mut Vec<CanonicalRange>) {
    if node.is_error() || node.is_missing() {
        out.push(mapper.byte_range_to_canonical(node.start_byte(), node.end_byte()));
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.has_error() || child.is_error() || child.is_missing() {
            collect_parse_errors(child, mapper, out);
        }
    }
}

fn extract_import_stmt(
    node: Node<'_>,
    src: &[u8],
    mapper: &PositionMapper,
    out: &mut Vec<ImportSpec>,
) {
    let range = mapper.byte_range_to_canonical(node.start_byte(), node.end_byte());
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "dotted_name" => {
                if let Ok(mod_name) = child.utf8_text(src) {
                    let local = mod_name.split('.').next().unwrap_or(mod_name).to_string();
                    out.push(ImportSpec {
                        local_name: local.clone(),
                        imported_name: local,
                        module_specifier: mod_name.to_string(),
                        is_type_only: false,
                        range,
                    });
                }
            }
            "aliased_import" => {
                let mod_name = child
                    .child_by_field_name("name")
                    .and_then(|n| n.utf8_text(src).ok())
                    .unwrap_or("");
                let alias = child
                    .child_by_field_name("alias")
                    .and_then(|n| n.utf8_text(src).ok())
                    .unwrap_or(mod_name);
                if !mod_name.is_empty() {
                    out.push(ImportSpec {
                        local_name: alias.to_string(),
                        imported_name: mod_name.to_string(),
                        module_specifier: mod_name.to_string(),
                        is_type_only: false,
                        range,
                    });
                }
            }
            _ => {}
        }
    }
}

fn extract_import_from_stmt(
    node: Node<'_>,
    src: &[u8],
    mapper: &PositionMapper,
    out: &mut Vec<ImportSpec>,
) {
    let range = mapper.byte_range_to_canonical(node.start_byte(), node.end_byte());
    let mod_spec = node
        .child_by_field_name("module_name")
        .and_then(|n| n.utf8_text(src).ok())
        .unwrap_or("")
        .to_string();
    if mod_spec.is_empty() {
        return;
    }

    let mut cursor = node.walk();
    let mut past_import_kw = false;
    for child in node.children(&mut cursor) {
        if child.kind() == "import" {
            past_import_kw = true;
            continue;
        }
        if !past_import_kw {
            continue;
        }
        match child.kind() {
            "dotted_name" | "identifier" => {
                if let Ok(name) = child.utf8_text(src) {
                    out.push(ImportSpec {
                        local_name: name.to_string(),
                        imported_name: name.to_string(),
                        module_specifier: mod_spec.clone(),
                        is_type_only: false,
                        range,
                    });
                }
            }
            "aliased_import" => {
                let name = child
                    .child_by_field_name("name")
                    .and_then(|n| n.utf8_text(src).ok())
                    .unwrap_or("");
                let alias = child
                    .child_by_field_name("alias")
                    .and_then(|n| n.utf8_text(src).ok())
                    .unwrap_or(name);
                if !name.is_empty() {
                    out.push(ImportSpec {
                        local_name: alias.to_string(),
                        imported_name: name.to_string(),
                        module_specifier: mod_spec.clone(),
                        is_type_only: false,
                        range,
                    });
                }
            }
            _ => {}
        }
    }
}

fn extract_fn_skeleton(
    node: Node<'_>,
    src: &[u8],
    mapper: &PositionMapper,
    class_prefix: Option<&str>,
) -> Option<FunctionSkeleton> {
    let name_node = node.child_by_field_name("name")?;
    let raw_name = name_node.utf8_text(src).ok()?.to_string();
    let name = match class_prefix {
        Some(cls) => format!("{cls}.{raw_name}"),
        None => raw_name.clone(),
    };
    let start = node.start_byte();
    let end = node.end_byte();
    let hash = xxhash_rust::xxh3::xxh3_64(&src[start..end]);
    Some(FunctionSkeleton {
        symbol_id: format!("{name}@{start}"),
        name,
        range: mapper.byte_range_to_canonical(start, end),
        start_byte: start,
        end_byte: end,
        content_hash: hash,
        is_exported: !raw_name.starts_with('_'),
    })
}

fn extract_class_methods(
    class_node: Node<'_>,
    src: &[u8],
    mapper: &PositionMapper,
    out: &mut Vec<FunctionSkeleton>,
) {
    let class_name = class_node
        .child_by_field_name("name")
        .and_then(|n| n.utf8_text(src).ok())
        .unwrap_or("Class");
    let Some(body) = class_node.child_by_field_name("body") else {
        return;
    };
    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        match child.kind() {
            "function_definition" => {
                if let Some(fskel) = extract_fn_skeleton(child, src, mapper, Some(class_name)) {
                    out.push(fskel);
                }
            }
            "decorated_definition" => {
                if let Some(def) = child.child_by_field_name("definition") {
                    if def.kind() == "function_definition" {
                        if let Some(fskel) = extract_fn_skeleton(def, src, mapper, Some(class_name))
                        {
                            out.push(fskel);
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

pub fn lower_py_function(
    _uri: &str,
    source: &str,
    skeleton: &FunctionSkeleton,
    imports: &[ImportSpec],
) -> FunctionIr {
    let mut parser = Parser::new();
    let lang = tree_sitter_python::LANGUAGE.into();
    let _ = parser.set_language(&lang);
    let tree = parser.parse(source, None);
    let mapper = PositionMapper::new(source);
    let src = source.as_bytes();

    let import_map: HashMap<String, ImportSpec> = imports
        .iter()
        .map(|i| (i.local_name.clone(), i.clone()))
        .collect();

    let mut builder = PyLowerer::new(src, &mapper, import_map);

    if let Some(tree) = tree {
        let root = tree.root_node();
        if skeleton.name == "<module>" {
            builder.lower_block_children(root);
        } else if let Some(fn_node) =
            find_node_by_span(root, skeleton.start_byte, skeleton.end_byte)
        {
            builder.lower_function_node(fn_node);
        }
    }

    builder.finish(skeleton)
}

fn find_node_by_span(node: Node<'_>, start: usize, end: usize) -> Option<Node<'_>> {
    if node.start_byte() == start && node.end_byte() == end && node.kind() == "function_definition"
    {
        return Some(node);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.start_byte() <= start && end <= child.end_byte() {
            if let Some(found) = find_node_by_span(child, start, end) {
                return Some(found);
            }
        }
    }
    None
}

struct PyLowerer<'a> {
    src: &'a [u8],
    mapper: &'a PositionMapper<'a>,
    imports: HashMap<String, ImportSpec>,
    blocks: Vec<BasicBlock>,
    cur_block: BlockId,
    insts: Vec<SsiInst>,
    env: HashMap<String, ValueId>,
    field_env: HashMap<(String, String), ValueId>,
    params: Vec<(String, ValueId)>,
    returns: Vec<ValueId>,
    danger_sinks: Vec<DangerSink>,
    span_index: Vec<(CanonicalRange, ValueId)>,
    return_absence: AbsenceValue,
    symbolic_callees: Vec<CallTarget>,
}

impl<'a> PyLowerer<'a> {
    fn new(src: &'a [u8], mapper: &'a PositionMapper, imports: HashMap<String, ImportSpec>) -> Self {
        let entry = BasicBlock {
            id: BlockId(0),
            preds: Vec::new(),
            succs: Vec::new(),
            insts: Vec::new(),
            guards: Vec::new(),
            terminated: false,
        };
        Self {
            src,
            mapper,
            imports,
            blocks: vec![entry],
            cur_block: BlockId(0),
            insts: Vec::new(),
            env: HashMap::new(),
            field_env: HashMap::new(),
            params: Vec::new(),
            returns: Vec::new(),
            danger_sinks: Vec::new(),
            span_index: Vec::new(),
            return_absence: AbsenceValue::bottom(),
            symbolic_callees: Vec::new(),
        }
    }

    fn new_block(&mut self) -> BlockId {
        let id = BlockId(self.blocks.len() as u32);
        self.blocks.push(BasicBlock {
            id,
            preds: Vec::new(),
            succs: Vec::new(),
            insts: Vec::new(),
            guards: Vec::new(),
            terminated: false,
        });
        id
    }

    fn connect(&mut self, from: BlockId, to: BlockId) {
        if !self.blocks[from.0 as usize].succs.contains(&to) {
            self.blocks[from.0 as usize].succs.push(to);
        }
        if !self.blocks[to.0 as usize].preds.contains(&from) {
            self.blocks[to.0 as usize].preds.push(from);
        }
    }

    fn emit(&mut self, name: Option<String>, node: Node<'_>, op: SsiOp) -> ValueId {
        let id = ValueId(self.insts.len() as u32);
        let range = self
            .mapper
            .byte_range_to_canonical(node.start_byte(), node.end_byte());
        let snippet = node
            .utf8_text(self.src)
            .ok()
            .map(|s| s.lines().next().unwrap_or("").trim().chars().take(80).collect());
        self.insts.push(SsiInst {
            id,
            block: self.cur_block,
            name,
            range,
            op,
            snippet,
        });
        self.blocks[self.cur_block.0 as usize].insts.push(id);
        self.span_index.push((range, id));
        id
    }

    fn lower_function_node(&mut self, fn_node: Node<'_>) {
        if let Some(params_node) = fn_node.child_by_field_name("parameters") {
            let mut cursor = params_node.walk();
            for (idx, child) in params_node.named_children(&mut cursor).enumerate() {
                self.lower_param(idx, child);
            }
        }
        if let Some(body) = fn_node.child_by_field_name("body") {
            self.lower_block_children(body);
        }
    }

    fn lower_param(&mut self, idx: usize, node: Node<'_>) {
        let mut param_name = None;
        let mut default_abs = AbsenceValue::present();

        match node.kind() {
            "identifier" => {
                param_name = node.utf8_text(self.src).ok().map(|s| s.to_string());
            }
            "default_parameter" => {
                param_name = node
                    .child_by_field_name("name")
                    .and_then(|n| n.utf8_text(self.src).ok())
                    .map(|s| s.to_string());
                if let Some(val) = node.child_by_field_name("value") {
                    if val.kind() == "none" {
                        default_abs = AbsenceValue::maybe_null();
                    }
                }
            }
            "typed_parameter" => {
                let mut c = node.walk();
                for ch in node.named_children(&mut c) {
                    if ch.kind() == "identifier" && param_name.is_none() {
                        param_name = ch.utf8_text(self.src).ok().map(|s| s.to_string());
                    }
                }
                if let Some(ty) = node.child_by_field_name("type") {
                    default_abs = classify_py_type_text(ty.utf8_text(self.src).unwrap_or(""));
                }
            }
            "typed_default_parameter" => {
                param_name = node
                    .child_by_field_name("name")
                    .and_then(|n| n.utf8_text(self.src).ok())
                    .map(|s| s.to_string());
                if let Some(ty) = node.child_by_field_name("type") {
                    default_abs = classify_py_type_text(ty.utf8_text(self.src).unwrap_or(""));
                }
                if let Some(val) = node.child_by_field_name("value") {
                    if val.kind() == "none" {
                        default_abs = default_abs.join(AbsenceValue::maybe_null());
                    }
                }
            }
            _ => {}
        }

        if let Some(name) = param_name {
            let vid = self.emit(
                Some(name.clone()),
                node,
                SsiOp::Param {
                    index: idx,
                    name: name.clone(),
                    default_absence: default_abs,
                },
            );
            self.env.insert(name.clone(), vid);
            self.params.push((name, vid));
        }
    }

    fn lower_block_children(&mut self, block_node: Node<'_>) {
        let mut cursor = block_node.walk();
        for child in block_node.named_children(&mut cursor) {
            if self.blocks[self.cur_block.0 as usize].terminated {
                break;
            }
            self.lower_stmt(child);
        }
    }

    fn lower_stmt(&mut self, node: Node<'_>) {
        if node.is_error() || node.is_missing() {
            self.emit(
                None,
                node,
                SsiOp::UnknownOp {
                    reason: UnknownReason::TreeSitterErrorNode,
                    symbol: "syntax_error".to_string(),
                },
            );
            return;
        }

        match node.kind() {
            "expression_statement" => {
                let mut c = node.walk();
                for ch in node.named_children(&mut c) {
                    self.lower_expr(ch);
                }
            }
            "return_statement" => {
                if let Some(arg) = node.named_child(0) {
                    let vid = self.lower_expr(arg);
                    self.returns.push(vid);
                    let abs = self.approx_value_absence(vid);
                    self.return_absence = self.return_absence.join(abs);
                } else {
                    let vid = self.emit(
                        None,
                        node,
                        SsiOp::Const {
                            value: AbsenceValue::null(),
                            label: "None".to_string(),
                        },
                    );
                    self.returns.push(vid);
                    self.return_absence = self.return_absence.join(AbsenceValue::null());
                }
                self.blocks[self.cur_block.0 as usize].terminated = true;
            }
            "raise_statement" => {
                self.blocks[self.cur_block.0 as usize].terminated = true;
                self.blocks[self.cur_block.0 as usize]
                    .guards
                    .push(GuardFact::Unreachable);
            }
            "break_statement" | "continue_statement" => {
                self.blocks[self.cur_block.0 as usize].terminated = true;
            }
            "assert_statement" => {
                if let Some(cond) = node.named_child(0) {
                    self.lower_expr(cond);
                    for (place, fact) in self.extract_py_guard_facts(cond, true) {
                        if let Some(&orig) = self.env.get(&place) {
                            let narrowed = self.emit(
                                Some(place.clone()),
                                node,
                                SsiOp::Pi {
                                    src: orig,
                                    guard: fact.clone(),
                                },
                            );
                            self.env.insert(place, narrowed);
                        }
                        self.blocks[self.cur_block.0 as usize].guards.push(fact);
                    }
                }
            }
            "if_statement" => self.lower_if_statement(node),
            "while_statement" | "for_statement" => {
                if let Some(body) = node.child_by_field_name("body") {
                    self.lower_block_children(body);
                }
            }
            "try_statement" => {
                let mut c = node.walk();
                for ch in node.named_children(&mut c) {
                    if ch.kind() == "block" {
                        self.lower_block_children(ch);
                    } else if matches!(ch.kind(), "except_clause" | "else_clause" | "finally_clause")
                    {
                        if let Some(b) = ch.named_children(&mut ch.walk()).find(|n| n.kind() == "block")
                        {
                            self.lower_block_children(b);
                        }
                    }
                }
            }
            _ => {
                if node.has_error() {
                    self.emit(
                        None,
                        node,
                        SsiOp::UnknownOp {
                            reason: UnknownReason::TreeSitterErrorNode,
                            symbol: "syntax_error".to_string(),
                        },
                    );
                }
            }
        }
    }

    fn lower_if_statement(&mut self, node: Node<'_>) {
        let Some(cond) = node.child_by_field_name("condition") else {
            return;
        };
        let _cond_vid = self.lower_expr(cond);
        let pre_env = self.env.clone();
        let pre_field_env = self.field_env.clone();
        let entry_block = self.cur_block;

        let then_block = self.new_block();
        let else_block = self.new_block();
        let merge_block = self.new_block();

        self.connect(entry_block, then_block);
        self.connect(entry_block, else_block);

        // THEN branch
        self.cur_block = then_block;
        self.env = pre_env.clone();
        self.field_env = pre_field_env.clone();
        for (place, fact) in self.extract_py_guard_facts(cond, true) {
            if let Some(&orig) = self.env.get(&place) {
                let narrowed = self.emit(
                    Some(place.clone()),
                    cond,
                    SsiOp::Pi {
                        src: orig,
                        guard: fact.clone(),
                    },
                );
                self.env.insert(place, narrowed);
            }
            self.blocks[then_block.0 as usize].guards.push(fact);
        }
        if let Some(consequence) = node.child_by_field_name("consequence") {
            self.lower_block_children(consequence);
        }
        let then_end = self.cur_block;
        let then_terminated = self.blocks[then_end.0 as usize].terminated;
        let then_env = self.env.clone();
        if !then_terminated {
            self.connect(then_end, merge_block);
        }

        // ELSE / ELIF branch
        self.cur_block = else_block;
        self.env = pre_env.clone();
        self.field_env = pre_field_env;
        for (place, fact) in self.extract_py_guard_facts(cond, false) {
            if let Some(&orig) = self.env.get(&place) {
                let narrowed = self.emit(
                    Some(place.clone()),
                    cond,
                    SsiOp::Pi {
                        src: orig,
                        guard: fact.clone(),
                    },
                );
                self.env.insert(place, narrowed);
            }
            self.blocks[else_block.0 as usize].guards.push(fact);
        }
        let mut cursor = node.walk();
        for alt in node.children_by_field_name("alternative", &mut cursor) {
            match alt.kind() {
                "else_clause" => {
                    if let Some(b) = alt.child_by_field_name("body") {
                        self.lower_block_children(b);
                    }
                }
                "elif_clause" => {
                    if let Some(b) = alt.child_by_field_name("consequence") {
                        self.lower_block_children(b);
                    }
                }
                _ => {}
            }
        }
        let else_end = self.cur_block;
        let else_terminated = self.blocks[else_end.0 as usize].terminated;
        let else_env = self.env.clone();
        if !else_terminated {
            self.connect(else_end, merge_block);
        }

        // MERGE
        self.cur_block = merge_block;
        if then_terminated && else_terminated {
            self.blocks[merge_block.0 as usize].terminated = true;
            self.env = pre_env;
        } else if then_terminated {
            self.env = else_env;
        } else if else_terminated {
            self.env = then_env;
        } else {
            let mut merged = pre_env.clone();
            for (var, &pre_v) in &pre_env {
                let tv = then_env.get(var).copied().unwrap_or(pre_v);
                let ev = else_env.get(var).copied().unwrap_or(pre_v);
                if tv != ev {
                    let phi = self.emit(
                        Some(var.clone()),
                        node,
                        SsiOp::Phi {
                            incoming: vec![(then_end, tv), (else_end, ev)],
                        },
                    );
                    merged.insert(var.clone(), phi);
                } else {
                    merged.insert(var.clone(), tv);
                }
            }
            for (var, &tv) in &then_env {
                if !merged.contains_key(var) {
                    if let Some(&ev) = else_env.get(var) {
                        let phi = self.emit(
                            Some(var.clone()),
                            node,
                            SsiOp::Phi {
                                incoming: vec![(then_end, tv), (else_end, ev)],
                            },
                        );
                        merged.insert(var.clone(), phi);
                    } else {
                        merged.insert(var.clone(), tv);
                    }
                }
            }
            self.env = merged;
        }
    }

    fn extract_py_guard_facts(&self, cond: Node<'_>, is_then: bool) -> Vec<(String, GuardFact)> {
        let mut facts = Vec::new();
        match cond.kind() {
            "identifier" => {
                if let Ok(name) = cond.utf8_text(self.src) {
                    if is_then {
                        // Truthiness narrows out None on `then`; `else` retains MaybeAbsent
                        facts.push((
                            name.to_string(),
                            GuardFact::StateIn {
                                place: name.to_string(),
                                mask: STATE_PRESENT,
                            },
                        ));
                    }
                }
            }
            "not_operator" => {
                if let Some(arg) = cond.child_by_field_name("argument") {
                    return self.extract_py_guard_facts(arg, !is_then);
                }
            }
            "parenthesized_expression" => {
                if let Some(inner) = cond.named_child(0) {
                    return self.extract_py_guard_facts(inner, is_then);
                }
            }
            "named_expression" => {
                if let Some(name_node) = cond.child_by_field_name("name") {
                    if let Ok(name) = name_node.utf8_text(self.src) {
                        if is_then {
                            facts.push((
                                name.to_string(),
                                GuardFact::StateIn {
                                    place: name.to_string(),
                                    mask: STATE_PRESENT,
                                },
                            ));
                        }
                    }
                }
            }
            "comparison_operator" => {
                let text = cond.utf8_text(self.src).unwrap_or("");
                let lhs = cond.named_child(0);
                let rhs = cond.named_child(1);
                if let (Some(l), Some(r)) = (lhs, rhs) {
                    let l_target = extract_walrus_or_ident(l, self.src);
                    let r_text = r.utf8_text(self.src).unwrap_or("").trim();
                    if let Some(var_name) = l_target {
                        if r_text == "None" {
                            let is_not_none = text.contains("is not") || text.contains("!=");
                            let is_eq_none = !is_not_none && (text.contains("is") || text.contains("=="));
                            if is_not_none {
                                let mask = if is_then { STATE_PRESENT } else { STATE_NULL };
                                facts.push((
                                    var_name.clone(),
                                    GuardFact::StateIn {
                                        place: var_name,
                                        mask,
                                    },
                                ));
                            } else if is_eq_none {
                                let mask = if is_then { STATE_NULL } else { STATE_PRESENT };
                                facts.push((
                                    var_name.clone(),
                                    GuardFact::StateIn {
                                        place: var_name,
                                        mask,
                                    },
                                ));
                            }
                        }
                    }
                }
            }
            _ => {}
        }
        facts
    }

    fn lower_expr(&mut self, node: Node<'_>) -> ValueId {
        if node.is_error() || node.is_missing() {
            return self.emit(
                None,
                node,
                SsiOp::UnknownOp {
                    reason: UnknownReason::TreeSitterErrorNode,
                    symbol: "syntax_error".to_string(),
                },
            );
        }

        match node.kind() {
            "none" => self.emit(
                None,
                node,
                SsiOp::Const {
                    value: AbsenceValue::null(),
                    label: "None".to_string(),
                },
            ),
            "true" | "false" | "integer" | "float" | "string" | "concatenated_string" | "list"
            | "dictionary" | "set" | "tuple" => self.emit(
                None,
                node,
                SsiOp::Const {
                    value: AbsenceValue::present(),
                    label: node.kind().to_string(),
                },
            ),
            "identifier" => {
                let name = node.utf8_text(self.src).unwrap_or("").to_string();
                if let Some(&vid) = self.env.get(&name) {
                    let range = self
                        .mapper
                        .byte_range_to_canonical(node.start_byte(), node.end_byte());
                    self.span_index.push((range, vid));
                    vid
                } else {
                    self.emit(
                        Some(name.clone()),
                        node,
                        SsiOp::Const {
                            value: AbsenceValue::present(),
                            label: name,
                        },
                    )
                }
            }
            "parenthesized_expression" => {
                if let Some(inner) = node.named_child(0) {
                    self.lower_expr(inner)
                } else {
                    self.emit(
                        None,
                        node,
                        SsiOp::Const {
                            value: AbsenceValue::present(),
                            label: "paren".to_string(),
                        },
                    )
                }
            }
            "comparison_operator" | "not_operator" | "boolean_operator" => {
                let mut c = node.walk();
                for ch in node.named_children(&mut c) {
                    self.lower_expr(ch);
                }
                self.emit(
                    None,
                    node,
                    SsiOp::Const {
                        value: AbsenceValue::present(),
                        label: node.kind().to_string(),
                    },
                )
            }
            "await" => {
                let inner = node
                    .named_child(0)
                    .map(|c| self.lower_expr(c))
                    .unwrap_or_else(|| {
                        self.emit(
                            None,
                            node,
                            SsiOp::Const {
                                value: AbsenceValue::present(),
                                label: "await".to_string(),
                            },
                        )
                    });
                // Suspension point invalidates mutable obj.field narrowings
                self.field_env.clear();
                inner
            }
            "assignment" | "augmented_assignment" => {
                let Some(left) = node.child_by_field_name("left") else {
                    return self.emit(
                        None,
                        node,
                        SsiOp::Const {
                            value: AbsenceValue::present(),
                            label: "assign".to_string(),
                        },
                    );
                };
                let rhs = node
                    .child_by_field_name("right")
                    .map(|r| self.lower_expr(r))
                    .unwrap_or_else(|| {
                        self.emit(
                            None,
                            node,
                            SsiOp::Const {
                                value: AbsenceValue::present(),
                                label: "assign".to_string(),
                            },
                        )
                    });
                if left.kind() == "attribute" {
                    if let (Some(obj), Some(attr)) = (
                        left.child_by_field_name("object"),
                        left.child_by_field_name("attribute"),
                    ) {
                        let obj_name = obj.utf8_text(self.src).unwrap_or("").to_string();
                        let field_name = attr.utf8_text(self.src).unwrap_or("").to_string();
                        let vid = self.emit(
                            Some(format!("{obj_name}.{field_name}")),
                            node,
                            SsiOp::Assign { src: rhs },
                        );
                        self.field_env.insert((obj_name, field_name), vid);
                        return vid;
                    }
                }
                let left_name = left.utf8_text(self.src).unwrap_or("").to_string();
                let vid = self.emit(Some(left_name.clone()), node, SsiOp::Assign { src: rhs });
                if !left_name.is_empty() {
                    self.field_env.retain(|(obj, _), _| obj != &left_name);
                    self.env.insert(left_name, vid);
                }
                vid
            }
            "named_expression" => {
                let name = node
                    .child_by_field_name("name")
                    .and_then(|n| n.utf8_text(self.src).ok())
                    .unwrap_or("")
                    .to_string();
                let val = node
                    .child_by_field_name("value")
                    .map(|v| self.lower_expr(v))
                    .unwrap_or_else(|| {
                        self.emit(
                            None,
                            node,
                            SsiOp::Const {
                                value: AbsenceValue::present(),
                                label: "walrus".to_string(),
                            },
                        )
                    });
                let vid = self.emit(Some(name.clone()), node, SsiOp::Assign { src: val });
                if !name.is_empty() {
                    self.env.insert(name, vid);
                }
                vid
            }
            "attribute" => {
                let obj = node.child_by_field_name("object");
                let attr = node.child_by_field_name("attribute");
                let field_name = attr
                    .and_then(|a| a.utf8_text(self.src).ok())
                    .unwrap_or("")
                    .to_string();
                if let Some(obj_node) = obj {
                    let obj_name = obj_node.utf8_text(self.src).unwrap_or("").to_string();
                    if let Some(&cached_field) =
                        self.field_env.get(&(obj_name.clone(), field_name.clone()))
                    {
                        return cached_field;
                    }
                    let base_vid = self.lower_expr(obj_node);
                    let range = self
                        .mapper
                        .byte_range_to_canonical(node.start_byte(), node.end_byte());
                    self.danger_sinks.push(DangerSink {
                        kind: DangerSinkKind::MemberAccess,
                        target: base_vid,
                        range,
                        label: format!("{obj_name}.{field_name}"),
                    });
                    return self.emit(
                        Some(field_name.clone()),
                        node,
                        SsiOp::ProjectField {
                            base: base_vid,
                            field: field_name,
                        },
                    );
                }
                self.emit(
                    None,
                    node,
                    SsiOp::Const {
                        value: AbsenceValue::present(),
                        label: "attribute".to_string(),
                    },
                )
            }
            "subscript" => {
                let base_vid = node
                    .child_by_field_name("value")
                    .map(|v| self.lower_expr(v))
                    .unwrap_or_else(|| {
                        self.emit(
                            None,
                            node,
                            SsiOp::Const {
                                value: AbsenceValue::present(),
                                label: "subscript".to_string(),
                            },
                        )
                    });
                let range = self
                    .mapper
                    .byte_range_to_canonical(node.start_byte(), node.end_byte());
                self.danger_sinks.push(DangerSink {
                    kind: DangerSinkKind::ElementIndex,
                    target: base_vid,
                    range,
                    label: node
                        .utf8_text(self.src)
                        .unwrap_or("[]")
                        .chars()
                        .take(32)
                        .collect(),
                });
                self.emit(
                    None,
                    node,
                    SsiOp::ProjectIndex {
                        base: base_vid,
                        index: None,
                    },
                )
            }
            "conditional_expression" => {
                let mut c = node.walk();
                let children: Vec<_> = node.named_children(&mut c).collect();
                let then_v = children.first().map(|n| self.lower_expr(*n));
                let else_v = children.get(2).map(|n| self.lower_expr(*n));
                match (then_v, else_v) {
                    (Some(t), Some(e)) => self.emit(
                        None,
                        node,
                        SsiOp::Phi {
                            incoming: vec![(self.cur_block, t), (self.cur_block, e)],
                        },
                    ),
                    (Some(t), None) => t,
                    _ => self.emit(
                        None,
                        node,
                        SsiOp::Const {
                            value: AbsenceValue::present(),
                            label: "ternary".to_string(),
                        },
                    ),
                }
            }
            "list_comprehension"
            | "set_comprehension"
            | "dictionary_comprehension"
            | "generator_expression" => {
                let mut c = node.walk();
                for ch in node.named_children(&mut c) {
                    if ch.kind() == "for_in_clause" {
                        if let Some(left) = ch.child_by_field_name("left") {
                            let name = left.utf8_text(self.src).unwrap_or("").to_string();
                            if !name.is_empty() {
                                let v = self.emit(
                                    Some(name.clone()),
                                    left,
                                    SsiOp::Const {
                                        value: AbsenceValue::present(),
                                        label: name.clone(),
                                    },
                                );
                                self.env.insert(name, v);
                            }
                        }
                    } else if ch.kind() == "if_clause" {
                        if let Some(cond) = ch.named_child(0) {
                            for (place, fact) in self.extract_py_guard_facts(cond, true) {
                                if let Some(&orig) = self.env.get(&place) {
                                    let narrowed = self.emit(
                                        Some(place.clone()),
                                        cond,
                                        SsiOp::Pi { src: orig, guard: fact },
                                    );
                                    self.env.insert(place, narrowed);
                                }
                            }
                        }
                    }
                }
                self.emit(
                    None,
                    node,
                    SsiOp::Const {
                        value: AbsenceValue::present(),
                        label: node.kind().to_string(),
                    },
                )
            }
            "call" => {
                let fn_node = node.child_by_field_name("function");
                let fn_text = fn_node
                    .and_then(|f| f.utf8_text(self.src).ok())
                    .unwrap_or("call")
                    .to_string();
                let tail = fn_text.rsplit('.').next().unwrap_or(&fn_text);

                let target = if lookup_python_stdlib(&fn_text).is_some()
                    || lookup_python_stdlib(tail).is_some()
                {
                    CallTarget::Stdlib(fn_text.clone())
                } else if let Some(imp) = self.imports.get(&fn_text).cloned() {
                    CallTarget::Imported {
                        module_specifier: imp.module_specifier,
                        symbol: imp.imported_name,
                    }
                } else if fn_node.map(|f| f.kind() == "identifier").unwrap_or(false) {
                    CallTarget::Local(fn_text.clone())
                } else {
                    CallTarget::Dynamic(fn_text.clone())
                };
                self.symbolic_callees.push(target.clone());

                let mut inputs = Vec::new();
                if let Some(args) = node.child_by_field_name("arguments") {
                    let mut ac = args.walk();
                    for arg in args.named_children(&mut ac) {
                        let arg_name = arg.utf8_text(self.src).unwrap_or("").to_string();
                        if !arg_name.is_empty() {
                            // Passing an object to a call may mutate its fields
                            self.field_env.retain(|(obj, _), _| obj != &arg_name);
                        }
                        inputs.push(self.lower_expr(arg));
                    }
                }

                self.emit(
                    Some(fn_text),
                    node,
                    SsiOp::Call {
                        callee: target,
                        inputs,
                        outputs: Vec::new(),
                        exit_edge: ExitEdge::Normal,
                    },
                )
            }
            _ => self.emit(
                None,
                node,
                SsiOp::Const {
                    value: AbsenceValue::present(),
                    label: node.kind().to_string(),
                },
            ),
        }
    }

    fn approx_value_absence(&self, vid: ValueId) -> AbsenceValue {
        let Some(inst) = self.insts.get(vid.0 as usize) else {
            return AbsenceValue::present();
        };
        match &inst.op {
            SsiOp::Const { value, .. } => *value,
            SsiOp::Param {
                default_absence, ..
            } => *default_absence,
            SsiOp::Assign { src } => self.approx_value_absence(*src),
            SsiOp::Pi {
                guard: GuardFact::StateIn { mask, .. },
                ..
            } => AbsenceValue {
                known_states: *mask,
                unknown_reasons: 0,
            },
            SsiOp::Pi { .. } => AbsenceValue::present(),
            SsiOp::Phi { incoming } => {
                let mut acc = AbsenceValue::bottom();
                for &(_b, v) in incoming {
                    acc = acc.join(self.approx_value_absence(v));
                }
                acc
            }
            SsiOp::Call {
                callee: CallTarget::Stdlib(sym),
                ..
            } => lookup_python_stdlib(sym).unwrap_or_else(AbsenceValue::present),
            SsiOp::UnknownOp { reason, .. } => AbsenceValue::unknown(*reason),
            _ => AbsenceValue::present(),
        }
    }

    fn finish(mut self, skeleton: &FunctionSkeleton) -> FunctionIr {
        self.span_index.sort_by_key(|(r, _)| (r.start, r.end));
        FunctionIr {
            symbol_id: skeleton.symbol_id.clone(),
            name: skeleton.name.clone(),
            range: skeleton.range,
            params: self.params,
            blocks: self.blocks,
            insts: self.insts,
            returns: self.returns,
            danger_sinks: self.danger_sinks,
            span_index: self.span_index,
            local_summary: FnSummary {
                return_absence: if self.return_absence == AbsenceValue::bottom() {
                    AbsenceValue::present()
                } else {
                    self.return_absence
                },
                param_flow_to_return: Vec::new(),
                symbolic_callees: self.symbolic_callees,
            },
        }
    }
}

fn extract_walrus_or_ident(node: Node<'_>, src: &[u8]) -> Option<String> {
    match node.kind() {
        "identifier" => node.utf8_text(src).ok().map(|s| s.to_string()),
        "parenthesized_expression" => node
            .named_child(0)
            .and_then(|c| extract_walrus_or_ident(c, src)),
        "named_expression" => node
            .child_by_field_name("name")
            .and_then(|n| n.utf8_text(src).ok())
            .map(|s| s.to_string()),
        _ => None,
    }
}
