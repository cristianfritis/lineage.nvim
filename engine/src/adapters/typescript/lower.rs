use std::collections::HashMap;

use tree_sitter::{Language, Node, Parser};

use crate::adapters::typescript::stdlib::lookup_stdlib_summary;
use crate::adapters::typescript::stub_extract::classify_type_annotation;
use crate::ir::guards::GuardFact;
use crate::ir::{
    AbsenceValue, BasicBlock, BlockId, CallTarget, DangerSink, DangerSinkKind, ExitEdge,
    ExportSpec, FileSkeleton, FnSummary, FunctionIr, FunctionSkeleton, ImportSpec, SsiInst, SsiOp,
    ValueId, STATE_NULL, STATE_PRESENT, STATE_UNDEFINED,
};
use crate::protocol::schema::{CanonicalRange, UnknownReason};
use crate::text::PositionMapper;

pub fn language_for_uri(uri: &str) -> Language {
    let lower = uri.to_ascii_lowercase();
    if lower.ends_with(".tsx") {
        tree_sitter_typescript::LANGUAGE_TSX.into()
    } else if lower.ends_with(".js")
        || lower.ends_with(".jsx")
        || lower.ends_with(".mjs")
        || lower.ends_with(".cjs")
    {
        tree_sitter_javascript::LANGUAGE.into()
    } else {
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
    }
}

pub fn parse_ts_skeleton(uri: &str, source: &str) -> FileSkeleton {
    let content_hash = xxhash_rust::xxh3::xxh3_64(source.as_bytes());
    let mut parser = Parser::new();
    let lang = language_for_uri(uri);
    if parser.set_language(&lang).is_err() {
        return FileSkeleton {
            uri: uri.to_string(),
            content_hash,
            imports: Vec::new(),
            exports: Vec::new(),
            functions: Vec::new(),
            parse_errors: Vec::new(),
        };
    }
    let Some(tree) = parser.parse(source, None) else {
        return FileSkeleton {
            uri: uri.to_string(),
            content_hash,
            imports: Vec::new(),
            exports: Vec::new(),
            functions: Vec::new(),
            parse_errors: Vec::new(),
        };
    };

    let mapper = PositionMapper::new(source);
    let src = source.as_bytes();
    let root = tree.root_node();
    let mut imports = Vec::new();
    let mut exports = Vec::new();
    let mut functions = Vec::new();
    let mut parse_errors = Vec::new();

    collect_parse_errors(root, &mapper, &mut parse_errors);

    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        match child.kind() {
            "import_statement" => {
                extract_import_stmt(child, src, &mapper, &mut imports);
            }
            "export_statement" => {
                extract_export_stmt(child, src, &mapper, &mut exports, &mut functions);
            }
            "function_declaration" | "generator_function_declaration" => {
                if let Some(fskel) = extract_fn_skeleton(child, src, &mapper, false) {
                    functions.push(fskel);
                }
            }
            "lexical_declaration" | "variable_declaration" => {
                extract_var_fn_skeletons(child, src, &mapper, false, &mut functions);
            }
            "class_declaration" => {
                extract_class_methods(child, src, &mapper, &mut functions);
            }
            _ => {}
        }
    }

    // Synthetic <module> scope for top-level bindings/expressions
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

fn collect_parse_errors(
    node: Node<'_>,
    mapper: &PositionMapper<'_>,
    out: &mut Vec<CanonicalRange>,
) {
    if node.is_error() || node.is_missing() {
        out.push(mapper.byte_range_to_canonical(node.start_byte(), node.end_byte()));
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_parse_errors(child, mapper, out);
    }
}

fn extract_import_stmt(
    node: Node<'_>,
    src: &[u8],
    mapper: &PositionMapper<'_>,
    imports: &mut Vec<ImportSpec>,
) {
    let Some(source_node) = node.child_by_field_name("source") else {
        return;
    };
    let module_specifier = strip_quotes(source_node.utf8_text(src).unwrap_or(""));
    let range = mapper.byte_range_to_canonical(node.start_byte(), node.end_byte());
    let is_type_only = node
        .utf8_text(src)
        .map(|t| t.trim_start().starts_with("import type"))
        .unwrap_or(false);

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "import_clause" {
            extract_import_clause(child, src, &module_specifier, is_type_only, range, imports);
        }
    }
}

fn extract_import_clause(
    clause: Node<'_>,
    src: &[u8],
    module_specifier: &str,
    is_type_only: bool,
    range: CanonicalRange,
    imports: &mut Vec<ImportSpec>,
) {
    let mut cursor = clause.walk();
    for child in clause.children(&mut cursor) {
        match child.kind() {
            "identifier" => {
                if let Ok(name) = child.utf8_text(src) {
                    imports.push(ImportSpec {
                        local_name: name.to_string(),
                        imported_name: "default".to_string(),
                        module_specifier: module_specifier.to_string(),
                        is_type_only,
                        range,
                    });
                }
            }
            "named_imports" => {
                let mut nc = child.walk();
                for spec in child.named_children(&mut nc) {
                    if spec.kind() == "import_specifier" {
                        let imported = spec
                            .child_by_field_name("name")
                            .and_then(|n| n.utf8_text(src).ok())
                            .unwrap_or("");
                        let local = spec
                            .child_by_field_name("alias")
                            .and_then(|n| n.utf8_text(src).ok())
                            .unwrap_or(imported);
                        if !local.is_empty() {
                            imports.push(ImportSpec {
                                local_name: local.to_string(),
                                imported_name: imported.to_string(),
                                module_specifier: module_specifier.to_string(),
                                is_type_only,
                                range,
                            });
                        }
                    }
                }
            }
            "namespace_import" => {
                if let Some(id) = child.named_child(0) {
                    if let Ok(name) = id.utf8_text(src) {
                        imports.push(ImportSpec {
                            local_name: name.to_string(),
                            imported_name: "*".to_string(),
                            module_specifier: module_specifier.to_string(),
                            is_type_only,
                            range,
                        });
                    }
                }
            }
            _ => {}
        }
    }
}

fn extract_export_stmt(
    node: Node<'_>,
    src: &[u8],
    mapper: &PositionMapper<'_>,
    exports: &mut Vec<ExportSpec>,
    functions: &mut Vec<FunctionSkeleton>,
) {
    let range = mapper.byte_range_to_canonical(node.start_byte(), node.end_byte());
    let text = node.utf8_text(src).unwrap_or("");
    let mod_spec = node
        .child_by_field_name("source")
        .and_then(|n| n.utf8_text(src).ok())
        .map(strip_quotes);

    if text.contains("export *") {
        exports.push(ExportSpec {
            exported_name: "*".to_string(),
            local_name: None,
            module_specifier: mod_spec,
            is_wildcard: true,
            range,
        });
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "function_declaration" | "generator_function_declaration" => {
                if let Some(fskel) = extract_fn_skeleton(child, src, mapper, true) {
                    exports.push(ExportSpec {
                        exported_name: fskel.name.clone(),
                        local_name: Some(fskel.name.clone()),
                        module_specifier: None,
                        is_wildcard: false,
                        range,
                    });
                    functions.push(fskel);
                }
            }
            "lexical_declaration" | "variable_declaration" => {
                extract_var_fn_skeletons(child, src, mapper, true, functions);
            }
            "class_declaration" => {
                extract_class_methods(child, src, mapper, functions);
            }
            _ => {}
        }
    }
}

fn extract_fn_skeleton(
    node: Node<'_>,
    src: &[u8],
    mapper: &PositionMapper<'_>,
    is_exported: bool,
) -> Option<FunctionSkeleton> {
    let name_node = node.child_by_field_name("name")?;
    let name = name_node.utf8_text(src).ok()?.to_string();
    let start_byte = node.start_byte();
    let end_byte = node.end_byte();
    let content_hash = xxhash_rust::xxh3::xxh3_64(&src[start_byte..end_byte]);
    Some(FunctionSkeleton {
        symbol_id: format!("{name}@{start_byte}"),
        name,
        range: mapper.byte_range_to_canonical(start_byte, end_byte),
        start_byte,
        end_byte,
        content_hash,
        is_exported,
    })
}

fn extract_var_fn_skeletons(
    decl: Node<'_>,
    src: &[u8],
    mapper: &PositionMapper<'_>,
    is_exported: bool,
    functions: &mut Vec<FunctionSkeleton>,
) {
    let mut cursor = decl.walk();
    for child in decl.named_children(&mut cursor) {
        if child.kind() == "variable_declarator" {
            let Some(name_node) = child.child_by_field_name("name") else {
                continue;
            };
            let Some(val_node) = child.child_by_field_name("value") else {
                continue;
            };
            if matches!(val_node.kind(), "arrow_function" | "function_expression") {
                if let Ok(name) = name_node.utf8_text(src) {
                    let start_byte = child.start_byte();
                    let end_byte = child.end_byte();
                    let content_hash = xxhash_rust::xxh3::xxh3_64(&src[start_byte..end_byte]);
                    functions.push(FunctionSkeleton {
                        symbol_id: format!("{name}@{start_byte}"),
                        name: name.to_string(),
                        range: mapper.byte_range_to_canonical(start_byte, end_byte),
                        start_byte,
                        end_byte,
                        content_hash,
                        is_exported,
                    });
                }
            }
        }
    }
}

fn extract_class_methods(
    class_node: Node<'_>,
    src: &[u8],
    mapper: &PositionMapper<'_>,
    functions: &mut Vec<FunctionSkeleton>,
) {
    let class_name = class_node
        .child_by_field_name("name")
        .and_then(|n| n.utf8_text(src).ok())
        .unwrap_or("Class");
    let Some(body) = class_node.child_by_field_name("body") else {
        return;
    };
    let mut cursor = body.walk();
    for child in body.named_children(&mut cursor) {
        if child.kind() == "method_definition" {
            if let Some(name_node) = child.child_by_field_name("name") {
                if let Ok(mname) = name_node.utf8_text(src) {
                    let start_byte = child.start_byte();
                    let end_byte = child.end_byte();
                    let content_hash = xxhash_rust::xxh3::xxh3_64(&src[start_byte..end_byte]);
                    functions.push(FunctionSkeleton {
                        symbol_id: format!("{class_name}.{mname}@{start_byte}"),
                        name: format!("{class_name}.{mname}"),
                        range: mapper.byte_range_to_canonical(start_byte, end_byte),
                        start_byte,
                        end_byte,
                        content_hash,
                        is_exported: false,
                    });
                }
            }
        }
    }
}

fn strip_quotes(s: &str) -> String {
    s.trim()
        .trim_matches(|c| c == '\'' || c == '"' || c == '`')
        .to_string()
}

/// Lowers a single function (or `<module>`) into Pruned SSI IR.
pub fn lower_ts_function(
    uri: &str,
    source: &str,
    skeleton: &FunctionSkeleton,
    imports: &[ImportSpec],
) -> FunctionIr {
    let mut parser = Parser::new();
    let lang = language_for_uri(uri);
    let _ = parser.set_language(&lang);
    let tree = parser.parse(source, None);
    let mapper = PositionMapper::new(source);
    let src = source.as_bytes();

    let mut ctx = LowerCtx::new(src, &mapper, imports);

    if let Some(ref t) = tree {
        let root = t.root_node();
        let target_node = if skeleton.name == "<module>" {
            Some(root)
        } else {
            find_node_at_byte_span(root, skeleton.start_byte, skeleton.end_byte)
        };

        if let Some(fn_node) = target_node {
            ctx.lower_function_node(fn_node, skeleton.name == "<module>");
        }
    }

    ctx.finish(skeleton)
}

fn find_node_at_byte_span<'a>(
    node: Node<'a>,
    start_byte: usize,
    end_byte: usize,
) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.start_byte() <= start_byte && end_byte <= child.end_byte() {
            if let Some(found) = find_node_at_byte_span(child, start_byte, end_byte) {
                return Some(found);
            }
        }
    }
    if node.kind() != "program" && node.start_byte() == start_byte && node.end_byte() == end_byte {
        return Some(node);
    }
    None
}

struct LowerCtx<'a> {
    src: &'a [u8],
    mapper: &'a PositionMapper<'a>,
    imports: HashMap<String, ImportSpec>,
    blocks: Vec<BasicBlock>,
    insts: Vec<SsiInst>,
    current_block: BlockId,
    env: HashMap<String, ValueId>,
    params: Vec<(String, ValueId)>,
    returns: Vec<ValueId>,
    danger_sinks: Vec<DangerSink>,
    span_index: Vec<(CanonicalRange, ValueId)>,
    return_absence: AbsenceValue,
    symbolic_callees: Vec<CallTarget>,
}

impl<'a> LowerCtx<'a> {
    fn new(src: &'a [u8], mapper: &'a PositionMapper<'a>, imports: &[ImportSpec]) -> Self {
        let mut import_map = HashMap::new();
        for imp in imports {
            import_map.insert(imp.local_name.clone(), imp.clone());
        }
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
            imports: import_map,
            blocks: vec![entry],
            insts: Vec::new(),
            current_block: BlockId(0),
            env: HashMap::new(),
            params: Vec::new(),
            returns: Vec::new(),
            danger_sinks: Vec::new(),
            span_index: Vec::new(),
            return_absence: AbsenceValue::bottom(),
            symbolic_callees: Vec::new(),
        }
    }

    fn new_block(&mut self, pred: BlockId) -> BlockId {
        let id = BlockId(self.blocks.len() as u32);
        self.blocks.push(BasicBlock {
            id,
            preds: vec![pred],
            succs: Vec::new(),
            insts: Vec::new(),
            guards: Vec::new(),
            terminated: false,
        });
        self.blocks[pred.0 as usize].succs.push(id);
        id
    }

    fn emit(&mut self, name: Option<String>, node: Node<'_>, op: SsiOp) -> ValueId {
        let id = ValueId(self.insts.len() as u32);
        let range = self
            .mapper
            .byte_range_to_canonical(node.start_byte(), node.end_byte());
        let snippet = node
            .utf8_text(self.src)
            .ok()
            .map(|s| s.lines().next().unwrap_or("").trim().to_string());
        self.insts.push(SsiInst {
            id,
            block: self.current_block,
            name,
            range,
            op,
            snippet,
        });
        self.blocks[self.current_block.0 as usize].insts.push(id);
        self.span_index.push((range, id));
        id
    }

    fn lower_function_node(&mut self, fn_node: Node<'_>, is_module: bool) {
        if !is_module {
            let actual_fn = if fn_node.kind() == "variable_declarator" {
                fn_node.child_by_field_name("value").unwrap_or(fn_node)
            } else {
                fn_node
            };
            if let Some(params_node) = actual_fn.child_by_field_name("parameters") {
                self.lower_params(params_node);
            }
            if let Some(body) = actual_fn.child_by_field_name("body") {
                self.lower_stmt(body);
            }
        } else {
            let mut cursor = fn_node.walk();
            for child in fn_node.named_children(&mut cursor) {
                self.lower_stmt(child);
            }
        }
    }

    fn lower_params(&mut self, params_node: Node<'_>) {
        let mut cursor = params_node.walk();
        let mut idx = 0usize;
        for child in params_node.named_children(&mut cursor) {
            if matches!(child.kind(), "required_parameter" | "optional_parameter") {
                let is_optional = child.kind() == "optional_parameter";
                let name = child
                    .child_by_field_name("pattern")
                    .or_else(|| child.child_by_field_name("name"))
                    .and_then(|n| n.utf8_text(self.src).ok())
                    .unwrap_or("param")
                    .to_string();
                let mut abs = if let Some(ty) = child.child_by_field_name("type") {
                    classify_type_annotation(ty, self.src)
                } else {
                    AbsenceValue::present()
                };
                if is_optional {
                    abs = abs.join(AbsenceValue::undefined());
                }
                let vid = self.emit(
                    Some(name.clone()),
                    child,
                    SsiOp::Param {
                        index: idx,
                        name: name.clone(),
                        default_absence: abs,
                    },
                );
                self.env.insert(name.clone(), vid);
                self.params.push((name, vid));
                idx += 1;
            } else if child.kind() == "identifier" {
                if let Ok(name) = child.utf8_text(self.src) {
                    let vid = self.emit(
                        Some(name.to_string()),
                        child,
                        SsiOp::Param {
                            index: idx,
                            name: name.to_string(),
                            default_absence: AbsenceValue::present(),
                        },
                    );
                    self.env.insert(name.to_string(), vid);
                    self.params.push((name.to_string(), vid));
                    idx += 1;
                }
            }
        }
    }

    fn lower_stmt(&mut self, node: Node<'_>) {
        if node.is_error() || node.is_missing() {
            let text = node.utf8_text(self.src).unwrap_or("syntax_error");
            self.emit(
                None,
                node,
                SsiOp::UnknownOp {
                    reason: UnknownReason::TreeSitterErrorNode,
                    symbol: text.chars().take(32).collect(),
                },
            );
            return;
        }

        match node.kind() {
            "statement_block" | "program" => {
                let mut cursor = node.walk();
                for child in node.named_children(&mut cursor) {
                    if self.blocks[self.current_block.0 as usize].terminated {
                        break;
                    }
                    self.lower_stmt(child);
                }
            }
            "lexical_declaration" | "variable_declaration" => {
                let mut cursor = node.walk();
                for child in node.named_children(&mut cursor) {
                    if child.kind() == "variable_declarator" {
                        if let Some(name_node) = child.child_by_field_name("name") {
                            let name = name_node.utf8_text(self.src).unwrap_or("_").to_string();
                            let val_id = if let Some(val_node) = child.child_by_field_name("value")
                            {
                                let rhs = self.lower_expr(val_node);
                                self.emit(Some(name.clone()), child, SsiOp::Assign { src: rhs })
                            } else {
                                self.emit(
                                    Some(name.clone()),
                                    child,
                                    SsiOp::Const {
                                        value: AbsenceValue::undefined(),
                                        label: format!("uninitialized {name}"),
                                    },
                                )
                            };
                            self.env.insert(name, val_id);
                        }
                    }
                }
            }
            "expression_statement" => {
                if let Some(expr) = node.named_child(0) {
                    self.lower_expr(expr);
                }
            }
            "return_statement" => {
                if let Some(expr) = node.named_child(0) {
                    let vid = self.lower_expr(expr);
                    self.returns.push(vid);
                    if let Some(inst) = self.insts.get(vid.0 as usize) {
                        if let SsiOp::Const { value, .. } = &inst.op {
                            self.return_absence = self.return_absence.join(*value);
                        }
                    }
                }
                self.blocks[self.current_block.0 as usize].terminated = true;
            }
            "throw_statement" => {
                if let Some(expr) = node.named_child(0) {
                    self.lower_expr(expr);
                }
                self.blocks[self.current_block.0 as usize].terminated = true;
            }
            "if_statement" => {
                self.lower_if(node);
            }
            _ => {
                let mut cursor = node.walk();
                for child in node.named_children(&mut cursor) {
                    if child.is_error() || child.is_missing() {
                        self.emit(
                            None,
                            child,
                            SsiOp::UnknownOp {
                                reason: UnknownReason::TreeSitterErrorNode,
                                symbol: "error".to_string(),
                            },
                        );
                    }
                }
            }
        }
    }

    fn lower_if(&mut self, node: Node<'_>) {
        let pre_block = self.current_block;
        let pre_env = self.env.clone();

        let (then_guard, else_guard) = node
            .child_by_field_name("condition")
            .map(|c| self.extract_condition_guards(c))
            .unwrap_or((None, None));

        // Then branch
        let then_block = self.new_block(pre_block);
        self.current_block = then_block;
        self.env = pre_env.clone();
        if let Some((var, mask)) = &then_guard {
            if let Some(&prev_vid) = self.env.get(var) {
                let fact = GuardFact::StateIn {
                    place: var.clone(),
                    mask: *mask,
                };
                self.blocks[then_block.0 as usize].guards.push(fact.clone());
                let pi_vid = self.emit(
                    Some(var.clone()),
                    node,
                    SsiOp::Pi {
                        src: prev_vid,
                        guard: fact,
                    },
                );
                self.env.insert(var.clone(), pi_vid);
            }
        }
        if let Some(conseq) = node.child_by_field_name("consequence") {
            self.lower_stmt(conseq);
        }
        let then_end = self.current_block;
        let then_terminated = self.blocks[then_end.0 as usize].terminated;
        let then_env = self.env.clone();

        // Else branch
        let else_block = self.new_block(pre_block);
        self.current_block = else_block;
        self.env = pre_env.clone();
        if let Some((var, mask)) = &else_guard {
            if let Some(&prev_vid) = self.env.get(var) {
                let fact = GuardFact::StateIn {
                    place: var.clone(),
                    mask: *mask,
                };
                self.blocks[else_block.0 as usize].guards.push(fact.clone());
                let pi_vid = self.emit(
                    Some(var.clone()),
                    node,
                    SsiOp::Pi {
                        src: prev_vid,
                        guard: fact,
                    },
                );
                self.env.insert(var.clone(), pi_vid);
            }
        }
        if let Some(alt) = node.child_by_field_name("alternative") {
            if let Some(inner) = alt.named_child(0) {
                self.lower_stmt(inner);
            } else {
                self.lower_stmt(alt);
            }
        }
        let else_end = self.current_block;
        let else_terminated = self.blocks[else_end.0 as usize].terminated;
        let else_env = self.env.clone();

        // Merge block (exclude terminated branches!)
        let merge_block = self.new_block(pre_block);
        self.current_block = merge_block;
        self.env = pre_env;

        match (then_terminated, else_terminated) {
            (true, true) => {
                self.blocks[merge_block.0 as usize].terminated = true;
            }
            (true, false) => {
                self.env = else_env;
            }
            (false, true) => {
                self.env = then_env;
            }
            (false, false) => {
                for (name, orig_vid) in self.env.clone() {
                    let tv = then_env.get(&name).copied().unwrap_or(orig_vid);
                    let ev = else_env.get(&name).copied().unwrap_or(orig_vid);
                    if tv != ev {
                        let phi = self.emit(
                            Some(name.clone()),
                            node,
                            SsiOp::Phi {
                                incoming: vec![(then_end, tv), (else_end, ev)],
                            },
                        );
                        self.env.insert(name, phi);
                    } else {
                        self.env.insert(name, tv);
                    }
                }
            }
        }
    }

    #[allow(clippy::type_complexity)]
    fn extract_condition_guards(
        &self,
        cond: Node<'_>,
    ) -> (Option<(String, u8)>, Option<(String, u8)>) {
        let mut inner = cond;
        if inner.kind() == "parenthesized_expression" {
            if let Some(c) = inner.named_child(0) {
                inner = c;
            }
        }
        let text = inner.utf8_text(self.src).unwrap_or("").trim();

        if inner.kind() == "binary_expression" {
            let left = inner
                .child_by_field_name("left")
                .and_then(|n| n.utf8_text(self.src).ok())
                .unwrap_or("")
                .trim();
            let right = inner
                .child_by_field_name("right")
                .and_then(|n| n.utf8_text(self.src).ok())
                .unwrap_or("")
                .trim();
            let op = inner
                .child_by_field_name("operator")
                .and_then(|n| n.utf8_text(self.src).ok())
                .unwrap_or("");

            let (var, lit) = if left == "null" || left == "undefined" {
                (right, left)
            } else {
                (left, right)
            };

            if !var.is_empty() {
                match (op, lit) {
                    ("!==", "null") => {
                        return (
                            Some((var.to_string(), STATE_PRESENT | STATE_UNDEFINED)),
                            Some((var.to_string(), STATE_NULL)),
                        );
                    }
                    ("!=", "null") | ("!=", "undefined") => {
                        return (
                            Some((var.to_string(), STATE_PRESENT)),
                            Some((var.to_string(), STATE_NULL | STATE_UNDEFINED)),
                        );
                    }
                    ("!==", "undefined") => {
                        return (
                            Some((var.to_string(), STATE_PRESENT | STATE_NULL)),
                            Some((var.to_string(), STATE_UNDEFINED)),
                        );
                    }
                    ("===", "null") => {
                        return (
                            Some((var.to_string(), STATE_NULL)),
                            Some((var.to_string(), STATE_PRESENT | STATE_UNDEFINED)),
                        );
                    }
                    ("==", "null") | ("==", "undefined") => {
                        return (
                            Some((var.to_string(), STATE_NULL | STATE_UNDEFINED)),
                            Some((var.to_string(), STATE_PRESENT)),
                        );
                    }
                    ("===", "undefined") => {
                        return (
                            Some((var.to_string(), STATE_UNDEFINED)),
                            Some((var.to_string(), STATE_PRESENT | STATE_NULL)),
                        );
                    }
                    _ => {}
                }
            }
        }

        // Truthiness check: `if (x)` -> then: Present; else: MaybeAbsent (asymmetry!)
        if inner.kind() == "identifier" {
            return (
                Some((text.to_string(), STATE_PRESENT)),
                Some((
                    text.to_string(),
                    STATE_PRESENT | STATE_NULL | STATE_UNDEFINED,
                )),
            );
        }

        // Negated truthiness: `if (!x)`
        if inner.kind() == "unary_expression" && text.starts_with('!') {
            let var = text.trim_start_matches('!').trim();
            if !var.is_empty() {
                return (
                    Some((
                        var.to_string(),
                        STATE_PRESENT | STATE_NULL | STATE_UNDEFINED,
                    )),
                    Some((var.to_string(), STATE_PRESENT)),
                );
            }
        }

        (None, None)
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
            "null" => self.emit(
                None,
                node,
                SsiOp::Const {
                    value: AbsenceValue::null(),
                    label: "null".to_string(),
                },
            ),
            "undefined" => self.emit(
                None,
                node,
                SsiOp::Const {
                    value: AbsenceValue::undefined(),
                    label: "undefined".to_string(),
                },
            ),
            "identifier" => {
                let name = node.utf8_text(self.src).unwrap_or("");
                if name == "undefined" {
                    return self.emit(
                        Some("undefined".to_string()),
                        node,
                        SsiOp::Const {
                            value: AbsenceValue::undefined(),
                            label: "undefined".to_string(),
                        },
                    );
                }
                if let Some(&vid) = self.env.get(name) {
                    let range = self
                        .mapper
                        .byte_range_to_canonical(node.start_byte(), node.end_byte());
                    self.span_index.push((range, vid));
                    vid
                } else if let Some(imp) = self.imports.get(name).cloned() {
                    let target = CallTarget::Imported {
                        module_specifier: imp.module_specifier,
                        symbol: imp.imported_name,
                    };
                    self.emit(
                        Some(name.to_string()),
                        node,
                        SsiOp::Call {
                            callee: target,
                            inputs: Vec::new(),
                            outputs: Vec::new(),
                            exit_edge: ExitEdge::Normal,
                        },
                    )
                } else {
                    self.emit(
                        Some(name.to_string()),
                        node,
                        SsiOp::Const {
                            value: AbsenceValue::present(),
                            label: name.to_string(),
                        },
                    )
                }
            }
            "number" | "string" | "template_string" | "true" | "false" | "object" | "array" => {
                let text = node.utf8_text(self.src).unwrap_or("literal");
                self.emit(
                    None,
                    node,
                    SsiOp::Const {
                        value: AbsenceValue::present(),
                        label: text.chars().take(24).collect(),
                    },
                )
            }
            "non_null_expression" => {
                let inner = node
                    .named_child(0)
                    .map(|c| self.lower_expr(c))
                    .unwrap_or_else(|| {
                        self.emit(
                            None,
                            node,
                            SsiOp::Const {
                                value: AbsenceValue::present(),
                                label: "!".to_string(),
                            },
                        )
                    });
                self.emit(
                    None,
                    node,
                    SsiOp::Pi {
                        src: inner,
                        guard: GuardFact::StateIn {
                            place: "_".to_string(),
                            mask: STATE_PRESENT,
                        },
                    },
                )
            }
            "await_expression" => {
                // Transparently unwrap Promise<T> return provenance
                if let Some(arg) = node.named_child(0) {
                    self.lower_expr(arg)
                } else {
                    self.emit(
                        None,
                        node,
                        SsiOp::Const {
                            value: AbsenceValue::present(),
                            label: "await".to_string(),
                        },
                    )
                }
            }
            "member_expression" => {
                let text = node.utf8_text(self.src).unwrap_or("");
                let is_optional = text.contains("?.");
                let obj = node.child_by_field_name("object");
                let prop = node
                    .child_by_field_name("property")
                    .and_then(|p| p.utf8_text(self.src).ok())
                    .unwrap_or("prop")
                    .to_string();
                let base_vid = obj.map(|o| self.lower_expr(o)).unwrap_or_else(|| {
                    self.emit(
                        None,
                        node,
                        SsiOp::Const {
                            value: AbsenceValue::present(),
                            label: "obj".to_string(),
                        },
                    )
                });
                let range = self
                    .mapper
                    .byte_range_to_canonical(node.start_byte(), node.end_byte());
                if !is_optional {
                    self.danger_sinks.push(DangerSink {
                        kind: DangerSinkKind::MemberAccess,
                        target: base_vid,
                        range,
                        label: text.chars().take(32).collect(),
                    });
                }
                let proj = self.emit(
                    Some(prop.clone()),
                    node,
                    SsiOp::ProjectField {
                        base: base_vid,
                        field: prop,
                    },
                );
                if is_optional {
                    let undef = self.emit(
                        None,
                        node,
                        SsiOp::Const {
                            value: AbsenceValue::undefined(),
                            label: "optional_chain".to_string(),
                        },
                    );
                    self.emit(
                        None,
                        node,
                        SsiOp::Phi {
                            incoming: vec![(self.current_block, proj), (self.current_block, undef)],
                        },
                    )
                } else {
                    proj
                }
            }
            "subscript_expression" => {
                let obj = node.child_by_field_name("object");
                let base_vid = obj.map(|o| self.lower_expr(o)).unwrap_or_else(|| {
                    self.emit(
                        None,
                        node,
                        SsiOp::Const {
                            value: AbsenceValue::present(),
                            label: "arr".to_string(),
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
                        .unwrap_or("index")
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
            "binary_expression" => {
                let op = node
                    .child_by_field_name("operator")
                    .and_then(|n| n.utf8_text(self.src).ok())
                    .unwrap_or("");
                if op == "??" {
                    let _left = node.child_by_field_name("left").map(|l| self.lower_expr(l));
                    let right = node
                        .child_by_field_name("right")
                        .map(|r| self.lower_expr(r))
                        .unwrap_or_else(|| {
                            self.emit(
                                None,
                                node,
                                SsiOp::Const {
                                    value: AbsenceValue::present(),
                                    label: "coalesce".to_string(),
                                },
                            )
                        });
                    return self.emit(None, node, SsiOp::Assign { src: right });
                }
                self.emit(
                    None,
                    node,
                    SsiOp::Const {
                        value: AbsenceValue::present(),
                        label: "expr".to_string(),
                    },
                )
            }
            "assignment_expression" | "augmented_assignment_expression" => {
                let left_name = node
                    .child_by_field_name("left")
                    .and_then(|l| l.utf8_text(self.src).ok())
                    .unwrap_or("")
                    .to_string();
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
                let vid = self.emit(Some(left_name.clone()), node, SsiOp::Assign { src: rhs });
                if !left_name.is_empty() {
                    self.env.insert(left_name, vid);
                }
                vid
            }
            "call_expression" => {
                let fn_node = node.child_by_field_name("function");
                let fn_text = fn_node
                    .and_then(|f| f.utf8_text(self.src).ok())
                    .unwrap_or("call")
                    .to_string();
                let tail = fn_text.rsplit('.').next().unwrap_or(&fn_text);

                let target = if lookup_stdlib_summary(&fn_text).is_some()
                    || lookup_stdlib_summary(tail).is_some()
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

    fn finish(mut self, skeleton: &FunctionSkeleton) -> FunctionIr {
        self.span_index.sort_by_key(|(r, _)| (r.start, r.end));
        let param_flow_to_return =
            crate::ir::compute_param_flow_to_return(&self.insts, &self.returns);
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
                return_absence: self.return_absence,
                param_flow_to_return,
                symbolic_callees: self.symbolic_callees,
            },
        }
    }
}
