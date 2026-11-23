use std::collections::HashMap;

use tree_sitter::{Node, Parser};

use crate::adapters::rust::stdlib::lookup_rust_stdlib;
use crate::ir::guards::GuardFact;
use crate::ir::{
    AbsenceValue, BasicBlock, BlockId, CallTarget, DangerSink, DangerSinkKind, ExitEdge,
    FileSkeleton, FnSummary, FunctionIr, FunctionSkeleton, ImportSpec, SsiInst, SsiOp, ValueId,
    STATE_NULL, STATE_PRESENT, STATE_UNDEFINED,
};
use crate::protocol::schema::{CanonicalRange, UnknownReason};
use crate::text::PositionMapper;

pub fn parse_rust_skeleton(uri: &str, source: &str) -> FileSkeleton {
    let content_hash = xxhash_rust::xxh3::xxh3_64(source.as_bytes());
    let mut parser = Parser::new();
    let lang = tree_sitter_rust::LANGUAGE.into();
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
            "use_declaration" => extract_use_decl(child, src, &mapper, &mut imports),
            "mod_item" => {
                // External `mod foo;` without inline body
                if child.child_by_field_name("body").is_none() {
                    if let Some(name) = child
                        .child_by_field_name("name")
                        .and_then(|n| n.utf8_text(src).ok())
                    {
                        imports.push(ImportSpec {
                            local_name: name.to_string(),
                            imported_name: name.to_string(),
                            module_specifier: format!("mod:{name}"),
                            is_type_only: false,
                            range: mapper.byte_range_to_canonical(child.start_byte(), child.end_byte()),
                        });
                    }
                }
            }
            "function_item" => {
                if let Some(fskel) = extract_fn_skeleton(child, src, &mapper, None) {
                    functions.push(fskel);
                }
            }
            "impl_item" => {
                let ty_name = child
                    .child_by_field_name("type")
                    .and_then(|n| n.utf8_text(src).ok())
                    .unwrap_or("Impl");
                if let Some(body) = child.child_by_field_name("body") {
                    let mut bc = body.walk();
                    for item in body.children(&mut bc) {
                        if item.kind() == "function_item" {
                            if let Some(fskel) =
                                extract_fn_skeleton(item, src, &mapper, Some(ty_name))
                            {
                                functions.push(fskel);
                            }
                        }
                    }
                }
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

fn extract_use_decl(
    node: Node<'_>,
    src: &[u8],
    mapper: &PositionMapper,
    out: &mut Vec<ImportSpec>,
) {
    let range = mapper.byte_range_to_canonical(node.start_byte(), node.end_byte());
    let text = node.utf8_text(src).unwrap_or("");
    let cleaned = text
        .trim_start_matches("pub ")
        .trim_start_matches("use ")
        .trim_end_matches(';')
        .trim();
    if cleaned.is_empty() {
        return;
    }
    // Handle `use crate::auth::{verify, Token}` and `use crate::auth::verify`
    if let Some((prefix, brace_part)) = cleaned.split_once('{') {
        let base = prefix.trim_end_matches("::");
        let inner = brace_part.trim_end_matches('}');
        for item in inner.split(',') {
            let sym = item.trim();
            if !sym.is_empty() {
                let (orig, local) = sym
                    .split_once(" as ")
                    .map(|(a, b)| (a.trim(), b.trim()))
                    .unwrap_or((sym, sym));
                out.push(ImportSpec {
                    local_name: local.to_string(),
                    imported_name: orig.to_string(),
                    module_specifier: format!("{base}::{orig}"),
                    is_type_only: false,
                    range,
                });
            }
        }
    } else {
        let (path_part, alias) = cleaned
            .split_once(" as ")
            .map(|(a, b)| (a.trim(), Some(b.trim())))
            .unwrap_or((cleaned, None));
        let orig = path_part.rsplit("::").next().unwrap_or(path_part);
        let local = alias.unwrap_or(orig);
        out.push(ImportSpec {
            local_name: local.to_string(),
            imported_name: orig.to_string(),
            module_specifier: path_part.to_string(),
            is_type_only: false,
            range,
        });
    }
}

fn extract_fn_skeleton(
    node: Node<'_>,
    src: &[u8],
    mapper: &PositionMapper,
    type_prefix: Option<&str>,
) -> Option<FunctionSkeleton> {
    let name_node = node.child_by_field_name("name")?;
    let raw_name = name_node.utf8_text(src).ok()?.to_string();
    let name = match type_prefix {
        Some(ty) => format!("{ty}::{raw_name}"),
        None => raw_name,
    };
    let start = node.start_byte();
    let end = node.end_byte();
    let hash = xxhash_rust::xxh3::xxh3_64(&src[start..end]);
    let is_exported = node
        .children(&mut node.walk())
        .any(|c| c.kind() == "visibility_modifier");
    Some(FunctionSkeleton {
        symbol_id: format!("{name}@{start}"),
        name,
        range: mapper.byte_range_to_canonical(start, end),
        start_byte: start,
        end_byte: end,
        content_hash: hash,
        is_exported,
    })
}

pub fn extract_rust_fn_signature(source: &str, symbol: &str) -> Option<AbsenceValue> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .ok()?;
    let tree = parser.parse(source, None)?;
    find_rust_fn_return(tree.root_node(), source.as_bytes(), symbol)
}

fn find_rust_fn_return(node: Node<'_>, src: &[u8], symbol: &str) -> Option<AbsenceValue> {
    if node.kind() == "function_item" {
        if let Some(name) = node
            .child_by_field_name("name")
            .and_then(|n| n.utf8_text(src).ok())
        {
            if name == symbol {
                if let Some(ret) = node.child_by_field_name("return_type") {
                    return Some(classify_rust_type_text(ret.utf8_text(src).unwrap_or("")));
                }
                return Some(AbsenceValue::present());
            }
        }
    }
    let mut c = node.walk();
    for ch in node.children(&mut c) {
        if let Some(found) = find_rust_fn_return(ch, src, symbol) {
            return Some(found);
        }
    }
    None
}

pub fn classify_rust_type_text(raw: &str) -> AbsenceValue {
    let t = raw.trim();
    if t.starts_with("Option<") || t.contains("::Option<") {
        AbsenceValue {
            known_states: STATE_PRESENT | STATE_NULL,
            unknown_reasons: 0,
        }
    } else if t.starts_with("Result<") || t.contains("::Result<") {
        AbsenceValue {
            known_states: STATE_PRESENT | STATE_UNDEFINED,
            unknown_reasons: 0,
        }
    } else {
        AbsenceValue::present()
    }
}

pub fn lower_rust_function(
    _uri: &str,
    source: &str,
    skeleton: &FunctionSkeleton,
    imports: &[ImportSpec],
) -> FunctionIr {
    let mut parser = Parser::new();
    let lang = tree_sitter_rust::LANGUAGE.into();
    let _ = parser.set_language(&lang);
    let tree = parser.parse(source, None);
    let mapper = PositionMapper::new(source);
    let src = source.as_bytes();

    let import_map: HashMap<String, ImportSpec> = imports
        .iter()
        .map(|i| (i.local_name.clone(), i.clone()))
        .collect();

    let mut builder = RustLowerer::new(src, &mapper, import_map);

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
    if node.start_byte() == start && node.end_byte() == end && node.kind() == "function_item" {
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

struct RustLowerer<'a> {
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

impl<'a> RustLowerer<'a> {
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
                match child.kind() {
                    "parameter" => {
                        let name = child
                            .child_by_field_name("pattern")
                            .and_then(|p| p.utf8_text(self.src).ok())
                            .map(|s| s.trim_start_matches("mut ").to_string());
                        let abs = child
                            .child_by_field_name("type")
                            .and_then(|t| t.utf8_text(self.src).ok())
                            .map(classify_rust_type_text)
                            .unwrap_or_else(AbsenceValue::present);
                        if let Some(n) = name {
                            let vid = self.emit(
                                Some(n.clone()),
                                child,
                                SsiOp::Param {
                                    index: idx,
                                    name: n.clone(),
                                    default_absence: abs,
                                },
                            );
                            self.env.insert(n.clone(), vid);
                            self.params.push((n, vid));
                        }
                    }
                    "self_parameter" => {
                        let vid = self.emit(
                            Some("self".to_string()),
                            child,
                            SsiOp::Param {
                                index: idx,
                                name: "self".to_string(),
                                default_absence: AbsenceValue::present(),
                            },
                        );
                        self.env.insert("self".to_string(), vid);
                        self.params.push(("self".to_string(), vid));
                    }
                    _ => {}
                }
            }
        }
        if let Some(body) = fn_node.child_by_field_name("body") {
            let tail = self.lower_block(body);
            if !self.blocks[self.cur_block.0 as usize].terminated {
                if let Some(t) = tail {
                    self.returns.push(t);
                    let abs = self.approx_value_absence(t);
                    self.return_absence = self.return_absence.join(abs);
                }
            }
        }
    }

    fn lower_block_children(&mut self, root: Node<'_>) {
        let mut cursor = root.walk();
        for child in root.named_children(&mut cursor) {
            if self.blocks[self.cur_block.0 as usize].terminated {
                break;
            }
            self.lower_stmt(child);
        }
    }

    fn lower_block(&mut self, block_node: Node<'_>) -> Option<ValueId> {
        let mut cursor = block_node.walk();
        let children: Vec<_> = block_node.named_children(&mut cursor).collect();
        let mut last_expr = None;
        for (i, child) in children.iter().enumerate() {
            if self.blocks[self.cur_block.0 as usize].terminated {
                break;
            }
            let is_last = i + 1 == children.len();
            if is_last && is_rust_expr_kind(child.kind()) {
                last_expr = Some(self.lower_expr(*child));
            } else {
                self.lower_stmt(*child);
            }
        }
        last_expr
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
            "let_declaration" => self.lower_let_declaration(node),
            "expression_statement" => {
                if let Some(expr) = node.named_child(0) {
                    if expr.kind() == "return_expression" {
                        self.lower_return(expr);
                    } else {
                        self.lower_expr(expr);
                    }
                }
            }
            "return_expression" => self.lower_return(node),
            "if_expression" => {
                self.lower_if_expression(node);
            }
            "match_expression" => {
                self.lower_match_expression(node);
            }
            _ => {
                if is_rust_expr_kind(node.kind()) {
                    self.lower_expr(node);
                } else if node.has_error() {
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

    fn lower_return(&mut self, node: Node<'_>) {
        if let Some(arg) = node.named_child(0) {
            let vid = self.lower_expr(arg);
            self.returns.push(vid);
            let abs = self.approx_value_absence(vid);
            self.return_absence = self.return_absence.join(abs);
        }
        self.blocks[self.cur_block.0 as usize].terminated = true;
    }

    fn lower_let_declaration(&mut self, node: Node<'_>) {
        let Some(pat) = node.child_by_field_name("pattern") else {
            return;
        };
        let Some(val_node) = node.child_by_field_name("value") else {
            return;
        };
        let rhs_vid = self.lower_expr(val_node);

        // Check `let Some(u) = expr else { ... };` or `let Ok(u) = expr else { ... };`
        if let Some(else_node) = node.child_by_field_name("alternative") {
            let pat_text = pat.utf8_text(self.src).unwrap_or("");
            if let Some((variant, bound)) = extract_variant_binding(pat_text) {
                let _ = else_node;
                let unwrapped = self.emit(
                    Some(bound.clone()),
                    pat,
                    SsiOp::UnwrapVariant {
                        base: rhs_vid,
                        variant,
                    },
                );
                let narrowed = self.emit(
                    Some(bound.clone()),
                    node,
                    SsiOp::Pi {
                        src: unwrapped,
                        guard: GuardFact::StateIn {
                            place: bound.clone(),
                            mask: STATE_PRESENT,
                        },
                    },
                );
                self.env.insert(bound, narrowed);
                return;
            }
        }

        let name = pat
            .utf8_text(self.src)
            .unwrap_or("")
            .trim_start_matches("mut ")
            .to_string();
        if !name.is_empty() {
            let vid = self.emit(Some(name.clone()), node, SsiOp::Assign { src: rhs_vid });
            self.field_env.retain(|(obj, _), _| obj != &name);
            self.env.insert(name, vid);
        }
    }

    fn lower_if_expression(&mut self, node: Node<'_>) -> ValueId {
        let Some(cond) = node.child_by_field_name("condition") else {
            return self.emit(
                None,
                node,
                SsiOp::Const {
                    value: AbsenceValue::present(),
                    label: "if".to_string(),
                },
            );
        };

        let pre_env = self.env.clone();
        let pre_field_env = self.field_env.clone();
        let entry_block = self.cur_block;

        let then_block = self.new_block();
        let else_block = self.new_block();
        let merge_block = self.new_block();

        self.connect(entry_block, then_block);
        self.connect(entry_block, else_block);

        // THEN
        self.cur_block = then_block;
        self.env = pre_env.clone();
        self.field_env = pre_field_env.clone();

        if cond.kind() == "let_condition" {
            if let (Some(pat), Some(val)) = (
                cond.child_by_field_name("pattern"),
                cond.child_by_field_name("value"),
            ) {
                let scrutinee = self.lower_expr(val);
                let pat_text = pat.utf8_text(self.src).unwrap_or("");
                if let Some((variant, bound)) = extract_variant_binding(pat_text) {
                    let unwrapped = self.emit(
                        Some(bound.clone()),
                        pat,
                        SsiOp::UnwrapVariant {
                            base: scrutinee,
                            variant,
                        },
                    );
                    let narrowed = self.emit(
                        Some(bound.clone()),
                        cond,
                        SsiOp::Pi {
                            src: unwrapped,
                            guard: GuardFact::StateIn {
                                place: bound.clone(),
                                mask: STATE_PRESENT,
                            },
                        },
                    );
                    self.env.insert(bound, narrowed);
                }
            }
        } else {
            let _ = self.lower_expr(cond);
            for (place, fact) in self.extract_rust_guard_facts(cond, true) {
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
        }

        let then_val = node
            .child_by_field_name("consequence")
            .and_then(|c| self.lower_block(c));
        let then_end = self.cur_block;
        let then_terminated = self.blocks[then_end.0 as usize].terminated;
        let then_env = self.env.clone();
        if !then_terminated {
            self.connect(then_end, merge_block);
        }

        // ELSE
        self.cur_block = else_block;
        self.env = pre_env.clone();
        self.field_env = pre_field_env;
        if cond.kind() != "let_condition" {
            for (place, fact) in self.extract_rust_guard_facts(cond, false) {
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
        }

        let mut else_val = None;
        if let Some(alt) = node.child_by_field_name("alternative") {
            if let Some(inner) = alt.named_child(0) {
                if inner.kind() == "block" {
                    else_val = self.lower_block(inner);
                } else if inner.kind() == "if_expression" {
                    else_val = Some(self.lower_if_expression(inner));
                }
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
            self.merge_envs(&pre_env, &then_env, then_end, &else_env, else_end, node);
        }

        match (then_val, else_val) {
            (Some(t), Some(e)) if !then_terminated && !else_terminated => self.emit(
                None,
                node,
                SsiOp::Phi {
                    incoming: vec![(then_end, t), (else_end, e)],
                },
            ),
            (Some(t), _) if !then_terminated => t,
            (_, Some(e)) if !else_terminated => e,
            _ => self.emit(
                None,
                node,
                SsiOp::Const {
                    value: AbsenceValue::present(),
                    label: "if".to_string(),
                },
            ),
        }
    }

    fn lower_match_expression(&mut self, node: Node<'_>) -> ValueId {
        let scrutinee = node
            .child_by_field_name("value")
            .map(|v| self.lower_expr(v))
            .unwrap_or_else(|| {
                self.emit(
                    None,
                    node,
                    SsiOp::Const {
                        value: AbsenceValue::present(),
                        label: "match".to_string(),
                    },
                )
            });

        let Some(body) = node.child_by_field_name("body") else {
            return scrutinee;
        };

        let pre_env = self.env.clone();
        let entry_block = self.cur_block;
        let merge_block = self.new_block();
        let mut arm_vals = Vec::new();

        let mut c = body.walk();
        for arm in body.named_children(&mut c) {
            if arm.kind() != "match_arm" {
                continue;
            }
            let arm_block = self.new_block();
            self.connect(entry_block, arm_block);
            self.cur_block = arm_block;
            self.env = pre_env.clone();

            let pat_text = arm
                .child_by_field_name("pattern")
                .and_then(|p| p.utf8_text(self.src).ok())
                .unwrap_or("");
            if let Some((variant, bound)) = extract_variant_binding(pat_text) {
                let unwrapped = self.emit(
                    Some(bound.clone()),
                    arm,
                    SsiOp::UnwrapVariant {
                        base: scrutinee,
                        variant,
                    },
                );
                let narrowed = self.emit(
                    Some(bound.clone()),
                    arm,
                    SsiOp::Pi {
                        src: unwrapped,
                        guard: GuardFact::StateIn {
                            place: bound.clone(),
                            mask: STATE_PRESENT,
                        },
                    },
                );
                self.env.insert(bound, narrowed);
            }

            let val_opt = arm.child_by_field_name("value").map(|v| {
                if v.kind() == "block" {
                    self.lower_block(v).unwrap_or_else(|| {
                        self.emit(
                            None,
                            v,
                            SsiOp::Const {
                                value: AbsenceValue::present(),
                                label: "arm".to_string(),
                            },
                        )
                    })
                } else {
                    self.lower_expr(v)
                }
            });

            let arm_end = self.cur_block;
            if !self.blocks[arm_end.0 as usize].terminated {
                self.connect(arm_end, merge_block);
                if let Some(v) = val_opt {
                    arm_vals.push((arm_end, v));
                }
            }
        }

        self.cur_block = merge_block;
        self.env = pre_env;
        if arm_vals.is_empty() {
            self.emit(
                None,
                node,
                SsiOp::Const {
                    value: AbsenceValue::present(),
                    label: "match".to_string(),
                },
            )
        } else if arm_vals.len() == 1 {
            arm_vals[0].1
        } else {
            self.emit(None, node, SsiOp::Phi { incoming: arm_vals })
        }
    }

    fn merge_envs(
        &mut self,
        pre_env: &HashMap<String, ValueId>,
        then_env: &HashMap<String, ValueId>,
        then_end: BlockId,
        else_env: &HashMap<String, ValueId>,
        else_end: BlockId,
        node: Node<'_>,
    ) {
        let mut merged = pre_env.clone();
        for (var, &pre_v) in pre_env {
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
        self.env = merged;
    }

    fn extract_rust_guard_facts(&self, cond: Node<'_>, is_then: bool) -> Vec<(String, GuardFact)> {
        let mut facts = Vec::new();
        let text = cond.utf8_text(self.src).unwrap_or("").trim();
        if let Some(recv) = text.strip_suffix(".is_some()") {
            let mask = if is_then { STATE_PRESENT } else { STATE_NULL };
            facts.push((
                recv.to_string(),
                GuardFact::StateIn {
                    place: recv.to_string(),
                    mask,
                },
            ));
        } else if let Some(recv) = text.strip_suffix(".is_none()") {
            let mask = if is_then { STATE_NULL } else { STATE_PRESENT };
            facts.push((
                recv.to_string(),
                GuardFact::StateIn {
                    place: recv.to_string(),
                    mask,
                },
            ));
        } else if let Some(recv) = text.strip_suffix(".is_ok()") {
            let mask = if is_then {
                STATE_PRESENT
            } else {
                STATE_UNDEFINED
            };
            facts.push((
                recv.to_string(),
                GuardFact::StateIn {
                    place: recv.to_string(),
                    mask,
                },
            ));
        } else if let Some(recv) = text.strip_suffix(".is_err()") {
            let mask = if is_then {
                STATE_UNDEFINED
            } else {
                STATE_PRESENT
            };
            facts.push((
                recv.to_string(),
                GuardFact::StateIn {
                    place: recv.to_string(),
                    mask,
                },
            ));
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
            "identifier" => {
                let name = node.utf8_text(self.src).unwrap_or("").to_string();
                if name == "None" {
                    return self.emit(
                        None,
                        node,
                        SsiOp::Const {
                            value: AbsenceValue::null(),
                            label: "None".to_string(),
                        },
                    );
                }
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
            "integer_literal" | "float_literal" | "string_literal" | "raw_string_literal"
            | "boolean_literal" | "char_literal" => self.emit(
                None,
                node,
                SsiOp::Const {
                    value: AbsenceValue::present(),
                    label: node.kind().to_string(),
                },
            ),
            "try_expression" => {
                let inner = node
                    .named_child(0)
                    .map(|c| self.lower_expr(c))
                    .unwrap_or_else(|| {
                        self.emit(
                            None,
                            node,
                            SsiOp::Const {
                                value: AbsenceValue::present(),
                                label: "try".to_string(),
                            },
                        )
                    });
                let unwrapped = self.emit(
                    None,
                    node,
                    SsiOp::UnwrapVariant {
                        base: inner,
                        variant: "Some|Ok".to_string(),
                    },
                );
                self.emit(
                    None,
                    node,
                    SsiOp::Pi {
                        src: unwrapped,
                        guard: GuardFact::StateIn {
                            place: "_".to_string(),
                            mask: STATE_PRESENT,
                        },
                    },
                )
            }
            "unsafe_block" => self.emit(
                None,
                node,
                SsiOp::UnknownOp {
                    reason: UnknownReason::DynamicDispatch,
                    symbol: "unsafe_block".to_string(),
                },
            ),
            "unary_expression" => {
                let text = node.utf8_text(self.src).unwrap_or("");
                if text.starts_with('*') {
                    // Raw pointer dereference inside/near unsafe
                    return self.emit(
                        None,
                        node,
                        SsiOp::UnknownOp {
                            reason: UnknownReason::DynamicDispatch,
                            symbol: "raw_ptr_deref".to_string(),
                        },
                    );
                }
                if let Some(arg) = node.named_child(0) {
                    self.lower_expr(arg)
                } else {
                    self.emit(
                        None,
                        node,
                        SsiOp::Const {
                            value: AbsenceValue::present(),
                            label: "unary".to_string(),
                        },
                    )
                }
            }
            "reference_expression" | "parenthesized_expression" => node
                .named_child(0)
                .map(|c| self.lower_expr(c))
                .unwrap_or_else(|| {
                    self.emit(
                        None,
                        node,
                        SsiOp::Const {
                            value: AbsenceValue::present(),
                            label: node.kind().to_string(),
                        },
                    )
                }),
            "await_expression" => {
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
                self.field_env.clear();
                inner
            }
            "assignment_expression" => {
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
                if left.kind() == "field_expression" {
                    if let (Some(obj), Some(field)) = (
                        left.child_by_field_name("value"),
                        left.child_by_field_name("field"),
                    ) {
                        let obj_name = obj.utf8_text(self.src).unwrap_or("").to_string();
                        let field_name = field.utf8_text(self.src).unwrap_or("").to_string();
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
            "field_expression" => {
                let obj = node.child_by_field_name("value");
                let field = node.child_by_field_name("field");
                let field_name = field
                    .and_then(|f| f.utf8_text(self.src).ok())
                    .unwrap_or("")
                    .to_string();
                if let Some(obj_node) = obj {
                    let obj_name = obj_node.utf8_text(self.src).unwrap_or("").to_string();
                    if let Some(&cached) =
                        self.field_env.get(&(obj_name.clone(), field_name.clone()))
                    {
                        return cached;
                    }
                    let base_vid = self.lower_expr(obj_node);
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
                        label: "field".to_string(),
                    },
                )
            }
            "if_expression" => self.lower_if_expression(node),
            "match_expression" => self.lower_match_expression(node),
            "call_expression" => self.lower_call_expression(node),
            "block" => self.lower_block(node).unwrap_or_else(|| {
                self.emit(
                    None,
                    node,
                    SsiOp::Const {
                        value: AbsenceValue::present(),
                        label: "block".to_string(),
                    },
                )
            }),
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

    fn lower_call_expression(&mut self, node: Node<'_>) -> ValueId {
        let fn_node = node.child_by_field_name("function");
        let fn_text = fn_node
            .and_then(|f| f.utf8_text(self.src).ok())
            .unwrap_or("call")
            .to_string();

        // Check `Some(x)` / `Ok(x)` / `Err(e)` constructors
        if fn_text == "Some" || fn_text == "Option::Some" || fn_text == "Ok" || fn_text == "Result::Ok"
        {
            return self.emit(
                Some(fn_text.clone()),
                node,
                SsiOp::Const {
                    value: AbsenceValue::present(),
                    label: fn_text,
                },
            );
        }
        if fn_text == "Err" || fn_text == "Result::Err" {
            return self.emit(
                Some(fn_text.clone()),
                node,
                SsiOp::Const {
                    value: AbsenceValue::undefined(),
                    label: fn_text,
                },
            );
        }

        // Handle method calls `recv.method(...)`
        if let Some(fnode) = fn_node {
            if fnode.kind() == "field_expression" {
                let recv_node = fnode.child_by_field_name("value");
                let method_name = fnode
                    .child_by_field_name("field")
                    .and_then(|m| m.utf8_text(self.src).ok())
                    .unwrap_or("");
                let recv_vid = recv_node.map(|r| self.lower_expr(r));
                let recv_name = recv_node
                    .and_then(|r| r.utf8_text(self.src).ok())
                    .unwrap_or("");

                if matches!(method_name, "unwrap" | "expect") {
                    if let Some(rv) = recv_vid {
                        let range = self
                            .mapper
                            .byte_range_to_canonical(node.start_byte(), node.end_byte());
                        self.danger_sinks.push(DangerSink {
                            kind: DangerSinkKind::Call,
                            target: rv,
                            range,
                            label: format!("{recv_name}.{method_name}()"),
                        });
                    }
                    return self.emit(
                        Some(fn_text),
                        node,
                        SsiOp::Const {
                            value: AbsenceValue::present(),
                            label: method_name.to_string(),
                        },
                    );
                }

                // Conservative mutability rule for shallow `obj.field`:
                // `&self` / read-only methods preserve narrowed `obj.field`;
                // `&mut` / mutating methods downgrade narrowed `obj.field` to Unknown.
                if is_mutating_method_heuristic(method_name) && !recv_name.is_empty() {
                    let keys: Vec<(String, String)> = self
                        .field_env
                        .keys()
                        .filter(|(o, _)| o == recv_name)
                        .cloned()
                        .collect();
                    for k in keys {
                        let unk = self.emit(
                            Some(format!("{}.{}", k.0, k.1)),
                            node,
                            SsiOp::UnknownOp {
                                reason: UnknownReason::DynamicDispatch,
                                symbol: format!("mut_call:{method_name}"),
                            },
                        );
                        self.field_env.insert(k, unk);
                    }
                }
            }
        }

        // Check arguments for `&mut arg` invalidation
        let mut inputs = Vec::new();
        if let Some(args) = node.child_by_field_name("arguments") {
            let mut ac = args.walk();
            for arg in args.named_children(&mut ac) {
                let arg_text = arg.utf8_text(self.src).unwrap_or("");
                if let Some(stripped) = arg_text.strip_prefix("&mut ") {
                    let obj = stripped.trim();
                    self.field_env.retain(|(o, _), _| o != obj);
                }
                inputs.push(self.lower_expr(arg));
            }
        }

        let tail = fn_text.rsplit("::").next().unwrap_or(&fn_text);
        let method = tail.rsplit('.').next().unwrap_or(tail);
        let target = if lookup_rust_stdlib(&fn_text).is_some()
            || lookup_rust_stdlib(method).is_some()
        {
            CallTarget::Stdlib(fn_text.clone())
        } else if let Some(imp) = self.imports.get(&fn_text).cloned() {
            CallTarget::Imported {
                module_specifier: imp.module_specifier,
                symbol: imp.imported_name,
            }
        } else if fn_text.contains("::") && (fn_text.starts_with("crate::") || fn_text.starts_with("super::") || fn_text.starts_with("self::")) {
            let sym = fn_text.rsplit("::").next().unwrap_or(&fn_text).to_string();
            CallTarget::Imported {
                module_specifier: fn_text.clone(),
                symbol: sym,
            }
        } else if fn_node.map(|f| f.kind() == "identifier").unwrap_or(false) {
            CallTarget::Local(fn_text.clone())
        } else {
            CallTarget::Dynamic(fn_text.clone())
        };
        self.symbolic_callees.push(target.clone());

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
            } => lookup_rust_stdlib(sym).unwrap_or_else(AbsenceValue::present),
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

fn is_rust_expr_kind(kind: &str) -> bool {
    matches!(
        kind,
        "identifier"
            | "integer_literal"
            | "float_literal"
            | "string_literal"
            | "raw_string_literal"
            | "boolean_literal"
            | "char_literal"
            | "call_expression"
            | "field_expression"
            | "try_expression"
            | "if_expression"
            | "match_expression"
            | "block"
            | "unsafe_block"
            | "unary_expression"
            | "reference_expression"
            | "parenthesized_expression"
            | "await_expression"
            | "assignment_expression"
            | "binary_expression"
    )
}

fn extract_variant_binding(pat: &str) -> Option<(String, String)> {
    let t = pat.trim();
    for prefix in ["Some(", "Option::Some(", "Ok(", "Result::Ok("] {
        if let Some(rest) = t.strip_prefix(prefix) {
            if let Some(inner) = rest.strip_suffix(')') {
                let bound = inner.trim().trim_start_matches("mut ").trim();
                if !bound.is_empty() && bound != "_" {
                    let variant = prefix.trim_end_matches('(').to_string();
                    return Some((variant, bound.to_string()));
                }
            }
        }
    }
    None
}

fn is_mutating_method_heuristic(method: &str) -> bool {
    matches!(
        method,
        "reset" | "clear" | "push" | "pop" | "insert" | "remove" | "set" | "update" | "drain"
    ) || method.starts_with("set_")
        || method.ends_with("_mut")
}
