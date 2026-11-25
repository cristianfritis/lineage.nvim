use std::collections::{HashMap, HashSet};

use tree_sitter::{Node, Parser};

use crate::adapters::go::stdlib::lookup_go_stdlib;
use crate::ir::guards::GuardFact;
use crate::ir::{
    compute_param_flow_to_return, AbsenceValue, BasicBlock, BlockId, CallTarget, DangerSink,
    DangerSinkKind, ExitEdge, FileSkeleton, FnSummary, FunctionIr, FunctionSkeleton, ImportSpec,
    SsiInst, SsiOp, ValueId, STATE_NULL, STATE_PRESENT,
};
use crate::protocol::schema::{CanonicalRange, UnknownReason};
use crate::text::PositionMapper;

pub fn parse_go_skeleton(uri: &str, source: &str) -> FileSkeleton {
    let content_hash = xxhash_rust::xxh3::xxh3_64(source.as_bytes());
    let mut parser = Parser::new();
    let lang = tree_sitter_go::LANGUAGE.into();
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
            "import_declaration" => extract_import_decl(child, src, &mapper, &mut imports),
            "function_declaration" => {
                if let Some(fskel) = extract_fn_skeleton(child, src, &mapper, None) {
                    functions.push(fskel);
                }
            }
            "method_declaration" => {
                let recv_type = extract_receiver_type(child, src);
                if let Some(fskel) = extract_fn_skeleton(child, src, &mapper, recv_type.as_deref())
                {
                    functions.push(fskel);
                }
            }
            _ => {}
        }
    }

    functions.push(FunctionSkeleton {
        symbol_id: "<module>@0".to_string(),
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
    let mut c = node.walk();
    for ch in node.children(&mut c) {
        collect_parse_errors(ch, mapper, out);
    }
}

fn extract_import_decl(
    node: Node<'_>,
    src: &[u8],
    mapper: &PositionMapper,
    out: &mut Vec<ImportSpec>,
) {
    let mut stack = vec![node];
    while let Some(cur) = stack.pop() {
        if cur.kind() == "import_spec" {
            let path_node = cur.child_by_field_name("path");
            if let Some(pn) = path_node {
                let raw_path = pn
                    .utf8_text(src)
                    .unwrap_or("")
                    .trim_matches('"')
                    .trim_matches('`')
                    .to_string();
                if !raw_path.is_empty() {
                    let alias = cur
                        .child_by_field_name("name")
                        .and_then(|n| n.utf8_text(src).ok())
                        .map(|s| s.trim().to_string());
                    let default_name = raw_path.rsplit('/').next().unwrap_or(&raw_path).to_string();
                    let local_name = match alias.as_deref() {
                        Some("." | "_") | None => default_name.clone(),
                        Some(a) => a.to_string(),
                    };
                    out.push(ImportSpec {
                        local_name,
                        imported_name: default_name,
                        module_specifier: raw_path,
                        is_type_only: false,
                        range: mapper.byte_range_to_canonical(cur.start_byte(), cur.end_byte()),
                    });
                }
            }
        } else {
            let mut c = cur.walk();
            for ch in cur.children(&mut c) {
                stack.push(ch);
            }
        }
    }
}

fn extract_receiver_type(node: Node<'_>, src: &[u8]) -> Option<String> {
    let recv = node.child_by_field_name("receiver")?;
    let text = recv.utf8_text(src).ok()?;
    let cleaned = text.trim_start_matches('(').trim_end_matches(')');
    let ty_part = cleaned.split_whitespace().last()?;
    Some(ty_part.trim_start_matches('*').to_string())
}

fn extract_fn_skeleton(
    node: Node<'_>,
    src: &[u8],
    mapper: &PositionMapper,
    recv_type: Option<&str>,
) -> Option<FunctionSkeleton> {
    let name_node = node.child_by_field_name("name")?;
    let raw_name = name_node.utf8_text(src).ok()?.to_string();
    let name = match recv_type {
        Some(ty) if !ty.is_empty() => format!("{ty}::{raw_name}"),
        _ => raw_name.clone(),
    };
    let start = node.start_byte();
    let end = node.end_byte();
    let hash = xxhash_rust::xxh3::xxh3_64(&src[start..end]);
    let is_exported = raw_name
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_uppercase());
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

pub fn lower_go_function(
    _uri: &str,
    source: &str,
    skeleton: &FunctionSkeleton,
    imports: &[ImportSpec],
) -> FunctionIr {
    let mut parser = Parser::new();
    let lang = tree_sitter_go::LANGUAGE.into();
    let mapper = PositionMapper::new(source);
    let src = source.as_bytes();

    if parser.set_language(&lang).is_ok() {
        if let Some(tree) = parser.parse(source, None) {
            if let Some(fn_node) =
                find_fn_node(tree.root_node(), skeleton.start_byte, skeleton.end_byte)
            {
                let mut builder = GoIrBuilder::new(src, &mapper, imports);
                builder.lower_function_node(fn_node);
                return builder.finish(skeleton);
            }
        }
    }

    let mut builder = GoIrBuilder::new(src, &mapper, imports);
    builder.ensure_block(BlockId(0));
    builder.finish(skeleton)
}

fn find_fn_node(root: Node<'_>, start: usize, end: usize) -> Option<Node<'_>> {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.start_byte() == start
            && node.end_byte() == end
            && matches!(
                node.kind(),
                "function_declaration" | "method_declaration" | "source_file"
            )
        {
            return Some(node);
        }
        let mut c = node.walk();
        for ch in node.children(&mut c) {
            if ch.start_byte() <= start && end <= ch.end_byte() {
                stack.push(ch);
            }
        }
    }
    None
}

struct GoIrBuilder<'a> {
    src: &'a [u8],
    mapper: &'a PositionMapper<'a>,
    imports: HashMap<String, ImportSpec>,
    blocks: Vec<BasicBlock>,
    insts: Vec<SsiInst>,
    params: Vec<(String, ValueId)>,
    named_returns: HashSet<String>,
    defer_mutated_return: bool,
    returns: Vec<ValueId>,
    danger_sinks: Vec<DangerSink>,
    span_index: Vec<(CanonicalRange, ValueId)>,
    symbolic_callees: Vec<CallTarget>,
    vars: HashMap<String, ValueId>,
    active_guards: Vec<GuardFact>,
    cur_block: BlockId,
    return_absence: AbsenceValue,
}

impl<'a> GoIrBuilder<'a> {
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
            params: Vec::new(),
            named_returns: HashSet::new(),
            defer_mutated_return: false,
            returns: Vec::new(),
            danger_sinks: Vec::new(),
            span_index: Vec::new(),
            symbolic_callees: Vec::new(),
            vars: HashMap::new(),
            active_guards: Vec::new(),
            cur_block: BlockId(0),
            return_absence: AbsenceValue::bottom(),
        }
    }

    fn text(&self, node: Node<'_>) -> String {
        node.utf8_text(self.src).unwrap_or("").to_string()
    }

    fn range(&self, node: Node<'_>) -> CanonicalRange {
        self.mapper
            .byte_range_to_canonical(node.start_byte(), node.end_byte())
    }

    fn snippet(&self, node: Node<'_>) -> Option<String> {
        let t = self.text(node);
        let first_line = t.lines().next().unwrap_or("").trim();
        if first_line.is_empty() {
            None
        } else {
            Some(first_line.chars().take(100).collect())
        }
    }

    fn ensure_block(&mut self, id: BlockId) {
        while self.blocks.len() <= id.0 as usize {
            let nid = BlockId(self.blocks.len() as u32);
            self.blocks.push(BasicBlock {
                id: nid,
                preds: Vec::new(),
                succs: Vec::new(),
                insts: Vec::new(),
                guards: Vec::new(),
                terminated: false,
            });
        }
    }

    fn new_block(&mut self, preds: Vec<BlockId>) -> BlockId {
        let id = BlockId(self.blocks.len() as u32);
        self.blocks.push(BasicBlock {
            id,
            preds: preds.clone(),
            succs: Vec::new(),
            insts: Vec::new(),
            guards: Vec::new(),
            terminated: false,
        });
        for p in preds {
            if let Some(pb) = self.blocks.get_mut(p.0 as usize) {
                if !pb.succs.contains(&id) {
                    pb.succs.push(id);
                }
            }
        }
        id
    }

    fn emit(&mut self, name: Option<String>, node: Node<'_>, op: SsiOp) -> ValueId {
        let id = ValueId(self.insts.len() as u32);
        let range = self.range(node);
        let inst = SsiInst {
            id,
            block: self.cur_block,
            name,
            range,
            op,
            snippet: self.snippet(node),
        };
        self.insts.push(inst);
        self.blocks[self.cur_block.0 as usize].insts.push(id);
        self.span_index.push((range, id));
        id
    }

    fn add_guard(&mut self, block: BlockId, fact: GuardFact) {
        self.blocks[block.0 as usize].guards.push(fact.clone());
        if block == self.cur_block {
            self.active_guards.push(fact);
        }
    }

    fn is_terminated(&self, block: BlockId) -> bool {
        self.blocks[block.0 as usize].terminated
    }

    fn mark_terminated(&mut self, block: BlockId) {
        self.blocks[block.0 as usize].terminated = true;
    }

    fn zero_value_for_type(&self, type_node: Option<Node<'_>>) -> AbsenceValue {
        let Some(tn) = type_node else {
            return AbsenceValue::present();
        };
        match tn.kind() {
            "pointer_type" | "slice_type" | "map_type" | "function_type" | "channel_type"
            | "interface_type" => AbsenceValue::null(),
            "type_identifier" => {
                let t = self.text(tn);
                if matches!(t.as_str(), "any" | "error") {
                    AbsenceValue::null()
                } else {
                    AbsenceValue::present()
                }
            }
            _ => AbsenceValue::present(),
        }
    }

    fn lower_function_node(&mut self, fn_node: Node<'_>) {
        if let Some(params_node) = fn_node.child_by_field_name("parameters") {
            let mut idx = 0usize;
            let mut c = params_node.walk();
            for ch in params_node.children(&mut c) {
                if ch.kind() == "parameter_declaration" {
                    let ty_node = ch.child_by_field_name("type");
                    let ty_text = ty_node.map(|t| self.text(t)).unwrap_or_default();
                    let default_abs = if ty_text.starts_with('*')
                        || ty_text.starts_with("[]")
                        || ty_text.starts_with("map[")
                        || ty_text == "error"
                        || ty_text == "any"
                        || ty_text.contains("interface")
                    {
                        AbsenceValue::maybe_null()
                    } else {
                        AbsenceValue::present()
                    };
                    let mut pc = ch.walk();
                    for sub in ch.children(&mut pc) {
                        if sub.kind() == "identifier" {
                            let pname = self.text(sub);
                            let vid = self.emit(
                                Some(pname.clone()),
                                sub,
                                SsiOp::Param {
                                    index: idx,
                                    name: pname.clone(),
                                    default_absence: default_abs,
                                },
                            );
                            self.params.push((pname.clone(), vid));
                            self.vars.insert(pname, vid);
                            idx += 1;
                        }
                    }
                }
            }
        }

        if let Some(res_node) = fn_node.child_by_field_name("result") {
            if res_node.kind() == "parameter_list" {
                let mut c = res_node.walk();
                for ch in res_node.children(&mut c) {
                    if ch.kind() == "parameter_declaration" {
                        let zero_abs = self.zero_value_for_type(ch.child_by_field_name("type"));
                        let mut pc = ch.walk();
                        for sub in ch.children(&mut pc) {
                            if sub.kind() == "identifier" {
                                let rname = self.text(sub);
                                self.named_returns.insert(rname.clone());
                                let vid = self.emit(
                                    Some(rname.clone()),
                                    sub,
                                    SsiOp::Const {
                                        value: zero_abs,
                                        label: format!("named_return {rname}"),
                                    },
                                );
                                self.vars.insert(rname, vid);
                            }
                        }
                    }
                }
            }
        }

        if let Some(body) = fn_node.child_by_field_name("body") {
            self.lower_block(body);
        } else {
            self.lower_block(fn_node);
        }

        if self.defer_mutated_return {
            self.return_absence = self
                .return_absence
                .with_unknown(UnknownReason::DynamicDispatch);
        }
    }

    fn lower_block(&mut self, block: Node<'_>) {
        let mut c = block.walk();
        for child in block.children(&mut c) {
            if self.is_terminated(self.cur_block) {
                break;
            }
            self.lower_stmt(child);
        }
    }

    fn lower_stmt(&mut self, stmt: Node<'_>) {
        if stmt.is_error() || stmt.is_missing() {
            self.emit(
                None,
                stmt,
                SsiOp::UnknownOp {
                    reason: UnknownReason::TreeSitterErrorNode,
                    symbol: "syntax_error".to_string(),
                },
            );
            return;
        }
        match stmt.kind() {
            "block" | "statement_list" => self.lower_block(stmt),
            "var_declaration" => {
                let mut c = stmt.walk();
                for ch in stmt.children(&mut c) {
                    if ch.kind() == "var_spec" {
                        self.lower_var_spec(ch);
                    }
                }
            }
            "short_var_declaration" => self.lower_short_var_decl(stmt),
            "assignment_statement" => self.lower_assignment(stmt),
            "if_statement" => self.lower_if(stmt),
            "expression_switch_statement" | "type_switch_statement" => {
                self.lower_switch(stmt);
            }
            "defer_statement" => {
                let body_text = self.text(stmt);
                let mutated: Vec<String> = self
                    .named_returns
                    .iter()
                    .filter(|nr| body_text.contains(nr.as_str()) && body_text.contains('='))
                    .cloned()
                    .collect();
                if !mutated.is_empty() {
                    self.defer_mutated_return = true;
                    for nr in mutated {
                        let unk = self.emit(
                            Some(nr.clone()),
                            stmt,
                            SsiOp::UnknownOp {
                                reason: UnknownReason::DynamicDispatch,
                                symbol: nr.clone(),
                            },
                        );
                        self.vars.insert(nr, unk);
                    }
                }
            }
            "go_statement" => {
                self.emit(
                    None,
                    stmt,
                    SsiOp::UnknownOp {
                        reason: UnknownReason::DynamicDispatch,
                        symbol: "go".to_string(),
                    },
                );
            }
            "return_statement" => self.lower_return(stmt),
            "expression_statement" => {
                if let Some(expr) = stmt.named_child(0) {
                    self.lower_expr(expr);
                }
            }
            _ => {
                if stmt.kind().ends_with("_statement") {
                    let mut c = stmt.walk();
                    for ch in stmt.named_children(&mut c) {
                        if ch.kind() == "block" {
                            self.lower_block(ch);
                        }
                    }
                }
            }
        }
    }

    fn lower_var_spec(&mut self, spec: Node<'_>) {
        let mut names = Vec::new();
        let mut c = spec.walk();
        for ch in spec.children(&mut c) {
            if ch.kind() == "identifier" {
                names.push((self.text(ch), ch));
            }
        }
        let type_node = spec.child_by_field_name("type");
        let val_node = spec.child_by_field_name("value");

        if let Some(vn) = val_node {
            let expr = vn.named_child(0).unwrap_or(vn);
            let rhs_vid = self.lower_expr(expr);
            for (name, n) in names {
                let vid = self.emit(Some(name.clone()), n, SsiOp::Assign { src: rhs_vid });
                self.vars.insert(name, vid);
            }
        } else {
            let zero_abs = self.zero_value_for_type(type_node);
            for (name, n) in names {
                let vid = self.emit(
                    Some(name.clone()),
                    n,
                    SsiOp::Const {
                        value: zero_abs,
                        label: format!("var {name}"),
                    },
                );
                self.vars.insert(name, vid);
            }
        }
    }

    fn collect_expr_list(node: Node<'_>) -> Vec<Node<'_>> {
        if node.kind() == "expression_list" {
            let mut out = Vec::new();
            let mut c = node.walk();
            for ch in node.named_children(&mut c) {
                out.push(ch);
            }
            out
        } else {
            vec![node]
        }
    }

    fn lower_short_var_decl(&mut self, node: Node<'_>) {
        let Some(lhs_node) = node.child_by_field_name("left") else {
            return;
        };
        let Some(rhs_node) = node.child_by_field_name("right") else {
            return;
        };
        let lhs_items = Self::collect_expr_list(lhs_node);
        let rhs_items = Self::collect_expr_list(rhs_node);

        if lhs_items.len() == 2 && rhs_items.len() == 1 {
            let val_name = self.text(lhs_items[0]);
            let second_name = self.text(lhs_items[1]);
            let rhs = rhs_items[0];
            match rhs.kind() {
                "index_expression" | "type_assertion_expression" => {
                    let v_vid = self.emit(
                        Some(val_name.clone()),
                        lhs_items[0],
                        SsiOp::Const {
                            value: AbsenceValue::maybe_null(),
                            label: format!("comma_ok {val_name}"),
                        },
                    );
                    let ok_vid = self.emit(
                        Some(second_name.clone()),
                        lhs_items[1],
                        SsiOp::Const {
                            value: AbsenceValue::present(),
                            label: format!("ok {second_name}"),
                        },
                    );
                    self.vars.insert(val_name.clone(), v_vid);
                    self.vars.insert(second_name.clone(), ok_vid);
                    self.add_guard(
                        self.cur_block,
                        GuardFact::Implies {
                            antecedent_place: second_name,
                            antecedent_mask: STATE_PRESENT,
                            consequent_place: val_name,
                            consequent_mask: STATE_PRESENT,
                        },
                    );
                    return;
                }
                "call_expression" => {
                    let call_vid = self.lower_call(rhs);
                    let val_vid = self.emit(
                        Some(val_name.clone()),
                        lhs_items[0],
                        SsiOp::Assign { src: call_vid },
                    );
                    let err_vid = self.emit(
                        Some(second_name.clone()),
                        lhs_items[1],
                        SsiOp::Const {
                            value: AbsenceValue::maybe_null(),
                            label: format!("err {second_name}"),
                        },
                    );
                    self.vars.insert(val_name.clone(), val_vid);
                    self.vars.insert(second_name.clone(), err_vid);
                    self.add_guard(
                        self.cur_block,
                        GuardFact::Implies {
                            antecedent_place: second_name,
                            antecedent_mask: STATE_NULL,
                            consequent_place: val_name,
                            consequent_mask: STATE_PRESENT,
                        },
                    );
                    return;
                }
                _ => {}
            }
        }

        for (i, lhs) in lhs_items.iter().enumerate() {
            let rhs_expr = rhs_items
                .get(i)
                .copied()
                .or_else(|| rhs_items.last().copied());
            if let Some(expr) = rhs_expr {
                let vid = self.lower_expr(expr);
                let name = self.text(*lhs);
                let bound = self.emit(Some(name.clone()), *lhs, SsiOp::Assign { src: vid });
                self.vars.insert(name, bound);
            }
        }
    }

    fn lower_assignment(&mut self, node: Node<'_>) {
        let Some(lhs_node) = node.child_by_field_name("left") else {
            return;
        };
        let Some(rhs_node) = node.child_by_field_name("right") else {
            return;
        };
        let lhs_items = Self::collect_expr_list(lhs_node);
        let rhs_items = Self::collect_expr_list(rhs_node);
        for (i, lhs) in lhs_items.iter().enumerate() {
            if let Some(&expr) = rhs_items.get(i).or_else(|| rhs_items.last()) {
                let vid = self.lower_expr(expr);
                let name = self.text(*lhs);
                let bound = self.emit(Some(name.clone()), *lhs, SsiOp::Assign { src: vid });
                self.vars.insert(name, bound);
            }
        }
    }

    fn parse_guard_condition(&self, cond: Node<'_>) -> (Option<GuardFact>, Option<GuardFact>) {
        let txt = self.text(cond);
        let t = txt.trim();
        if let Some((lhs, rhs)) = t.split_once("!=") {
            let l = lhs.trim();
            let r = rhs.trim();
            let place = if r == "nil" {
                l
            } else if l == "nil" {
                r
            } else {
                ""
            };
            if !place.is_empty() {
                return (
                    Some(GuardFact::StateIn {
                        place: place.to_string(),
                        mask: STATE_PRESENT,
                    }),
                    Some(GuardFact::StateIn {
                        place: place.to_string(),
                        mask: STATE_NULL,
                    }),
                );
            }
        }
        if let Some((lhs, rhs)) = t.split_once("==") {
            let l = lhs.trim();
            let r = rhs.trim();
            let place = if r == "nil" {
                l
            } else if l == "nil" {
                r
            } else {
                ""
            };
            if !place.is_empty() {
                return (
                    Some(GuardFact::StateIn {
                        place: place.to_string(),
                        mask: STATE_NULL,
                    }),
                    Some(GuardFact::StateIn {
                        place: place.to_string(),
                        mask: STATE_PRESENT,
                    }),
                );
            }
        }
        if let Some(rest) = t.strip_prefix('!') {
            let place = rest.trim();
            if !place.is_empty() && place.chars().all(|c| c.is_alphanumeric() || c == '_') {
                return (
                    Some(GuardFact::StateIn {
                        place: place.to_string(),
                        mask: STATE_NULL,
                    }),
                    Some(GuardFact::StateIn {
                        place: place.to_string(),
                        mask: STATE_PRESENT,
                    }),
                );
            }
        }
        if t.chars().all(|c| c.is_alphanumeric() || c == '_') {
            return (
                Some(GuardFact::StateIn {
                    place: t.to_string(),
                    mask: STATE_PRESENT,
                }),
                Some(GuardFact::StateIn {
                    place: t.to_string(),
                    mask: STATE_NULL,
                }),
            );
        }
        (None, None)
    }

    fn lower_if(&mut self, node: Node<'_>) {
        if let Some(init) = node.child_by_field_name("initializer") {
            self.lower_stmt(init);
        }
        let cond_node = node.child_by_field_name("condition");
        let (then_guard, else_guard) = cond_node
            .map(|c| self.parse_guard_condition(c))
            .unwrap_or((None, None));

        let pre_block = self.cur_block;
        let pre_vars = self.vars.clone();
        let pre_guards = self.active_guards.clone();

        let then_block = self.new_block(vec![pre_block]);
        self.cur_block = then_block;
        self.active_guards = pre_guards.clone();
        if let Some(ref g) = then_guard {
            self.add_guard(then_block, g.clone());
        }
        if let Some(conseq) = node.child_by_field_name("consequence") {
            self.lower_block(conseq);
        }
        let then_term = self.is_terminated(self.cur_block);
        let then_end = self.cur_block;
        let then_vars = self.vars.clone();

        let else_block = self.new_block(vec![pre_block]);
        self.cur_block = else_block;
        self.vars = pre_vars.clone();
        self.active_guards = pre_guards.clone();
        if let Some(ref g) = else_guard {
            self.add_guard(else_block, g.clone());
        }
        if let Some(alt) = node.child_by_field_name("alternative") {
            self.lower_stmt(alt);
        }
        let else_term = self.is_terminated(self.cur_block);
        let else_end = self.cur_block;
        let else_vars = self.vars.clone();

        let mut join_preds = Vec::new();
        if !then_term {
            join_preds.push(then_end);
        }
        if !else_term {
            join_preds.push(else_end);
        }
        if join_preds.is_empty() {
            self.mark_terminated(pre_block);
            return;
        }
        let join_block = self.new_block(join_preds);
        self.cur_block = join_block;

        if then_term && !else_term {
            self.vars = else_vars;
            self.active_guards = pre_guards;
            if let Some(g) = else_guard {
                self.add_guard(join_block, g);
            }
        } else if !then_term && else_term {
            self.vars = then_vars;
            self.active_guards = pre_guards;
            if let Some(g) = then_guard {
                self.add_guard(join_block, g);
            }
        } else {
            self.active_guards = pre_guards;
            let all_keys: HashSet<String> = pre_vars
                .keys()
                .chain(then_vars.keys())
                .chain(else_vars.keys())
                .cloned()
                .collect();
            let mut merged = HashMap::new();
            for k in all_keys {
                let tv = then_vars.get(&k).copied();
                let ev = else_vars.get(&k).copied();
                match (tv, ev) {
                    (Some(a), Some(b)) if a == b => {
                        merged.insert(k, a);
                    }
                    (Some(a), Some(b)) => {
                        let phi = self.emit(
                            Some(k.clone()),
                            node,
                            SsiOp::Phi {
                                incoming: vec![(then_end, a), (else_end, b)],
                            },
                        );
                        merged.insert(k, phi);
                    }
                    (Some(a), None) | (None, Some(a)) => {
                        merged.insert(k, a);
                    }
                    (None, None) => {}
                }
            }
            self.vars = merged;
        }
    }

    fn lower_switch(&mut self, node: Node<'_>) {
        let pre_block = self.cur_block;
        let pre_vars = self.vars.clone();
        let pre_guards = self.active_guards.clone();

        let mut case_ends = Vec::new();
        let mut case_var_maps = Vec::new();
        let mut c = node.walk();
        for ch in node.children(&mut c) {
            if matches!(ch.kind(), "expression_case" | "type_case" | "default_case") {
                let cb = self.new_block(vec![pre_block]);
                self.cur_block = cb;
                self.vars = pre_vars.clone();
                self.active_guards = pre_guards.clone();
                let mut sc = ch.walk();
                for sub in ch.children(&mut sc) {
                    if sub.kind() == "statement_list" {
                        self.lower_block(sub);
                    } else if sub.is_named()
                        && !matches!(sub.kind(), "expression_list" | "type_list")
                    {
                        self.lower_stmt(sub);
                    }
                }
                if !self.is_terminated(self.cur_block) {
                    case_ends.push(self.cur_block);
                    case_var_maps.push(self.vars.clone());
                }
            }
        }

        if case_ends.is_empty() {
            let join = self.new_block(vec![pre_block]);
            self.cur_block = join;
            self.vars = pre_vars;
            self.active_guards = pre_guards;
            return;
        }

        let join = self.new_block(case_ends.clone());
        self.cur_block = join;
        self.active_guards = pre_guards;
        let mut merged = pre_vars.clone();
        for k in pre_vars.keys() {
            let mut incoming = Vec::new();
            for (idx, vm) in case_var_maps.iter().enumerate() {
                if let Some(&vid) = vm.get(k) {
                    incoming.push((case_ends[idx], vid));
                }
            }
            if incoming.len() > 1 && incoming.windows(2).any(|w| w[0].1 != w[1].1) {
                let phi = self.emit(Some(k.clone()), node, SsiOp::Phi { incoming });
                merged.insert(k.clone(), phi);
            } else if let Some(&(_, v)) = incoming.first() {
                merged.insert(k.clone(), v);
            }
        }
        self.vars = merged;
    }

    fn lower_return(&mut self, node: Node<'_>) {
        let mut c = node.walk();
        let mut exprs = Vec::new();
        for ch in node.named_children(&mut c) {
            if ch.kind() == "expression_list" {
                exprs.extend(Self::collect_expr_list(ch));
            } else {
                exprs.push(ch);
            }
        }
        if exprs.is_empty() && !self.named_returns.is_empty() {
            for nr in self.named_returns.clone() {
                if let Some(&vid) = self.vars.get(&nr) {
                    self.returns.push(vid);
                    self.return_absence = self.return_absence.join(self.approx_value_absence(vid));
                }
            }
        } else {
            for e in exprs {
                let vid = self.lower_expr(e);
                self.returns.push(vid);
                self.return_absence = self.return_absence.join(self.approx_value_absence(vid));
            }
        }
        self.mark_terminated(self.cur_block);
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
            "nil" => self.emit(
                Some("nil".to_string()),
                node,
                SsiOp::Const {
                    value: AbsenceValue::null(),
                    label: "nil".to_string(),
                },
            ),
            "int_literal"
            | "float_literal"
            | "interpreted_string_literal"
            | "raw_string_literal"
            | "rune_literal"
            | "true"
            | "false"
            | "composite_literal"
            | "func_literal" => self.emit(
                None,
                node,
                SsiOp::Const {
                    value: AbsenceValue::present(),
                    label: node.kind().to_string(),
                },
            ),
            "parenthesized_expression" => node
                .named_child(0)
                .map(|c| self.lower_expr(c))
                .unwrap_or_else(|| {
                    self.emit(
                        None,
                        node,
                        SsiOp::Const {
                            value: AbsenceValue::present(),
                            label: "paren".to_string(),
                        },
                    )
                }),
            "identifier" => {
                let name = self.text(node);
                if let Some(&vid) = self.vars.get(&name) {
                    let has_relevant_guard = self.active_guards.iter().any(|g| match g {
                        GuardFact::StateIn { place, .. } => place == &name,
                        GuardFact::Implies {
                            consequent_place, ..
                        } => consequent_place == &name,
                        GuardFact::Unreachable => false,
                    });
                    if has_relevant_guard {
                        let narrowed = self.emit(
                            Some(name.clone()),
                            node,
                            SsiOp::Pi {
                                src: vid,
                                guard: GuardFact::StateIn {
                                    place: name.clone(),
                                    mask: STATE_PRESENT | STATE_NULL,
                                },
                            },
                        );
                        self.vars.insert(name, narrowed);
                        return narrowed;
                    }
                    return vid;
                }
                self.emit(
                    Some(name.clone()),
                    node,
                    SsiOp::UnknownOp {
                        reason: UnknownReason::ExternalUntyped,
                        symbol: name,
                    },
                )
            }
            "unary_expression" => {
                let op = node
                    .child_by_field_name("operator")
                    .map(|o| self.text(o))
                    .unwrap_or_default();
                let operand = node.child_by_field_name("operand");
                if op == "<-" {
                    return self.emit(
                        None,
                        node,
                        SsiOp::UnknownOp {
                            reason: UnknownReason::DynamicDispatch,
                            symbol: "chan_recv".to_string(),
                        },
                    );
                }
                if let Some(inner) = operand {
                    let base = self.lower_expr(inner);
                    if op == "*" {
                        self.danger_sinks.push(DangerSink {
                            kind: DangerSinkKind::MemberAccess,
                            target: base,
                            range: self.range(node),
                            label: format!("*{}", self.text(inner)),
                        });
                    }
                    return base;
                }
                self.emit(
                    None,
                    node,
                    SsiOp::Const {
                        value: AbsenceValue::present(),
                        label: "unary".to_string(),
                    },
                )
            }
            "selector_expression" => {
                let base_vid = node
                    .child_by_field_name("operand")
                    .map(|o| self.lower_expr(o))
                    .unwrap_or_else(|| {
                        self.emit(
                            None,
                            node,
                            SsiOp::UnknownOp {
                                reason: UnknownReason::ExternalUntyped,
                                symbol: self.text(node),
                            },
                        )
                    });
                let field = node
                    .child_by_field_name("field")
                    .map(|f| self.text(f))
                    .unwrap_or_default();
                self.danger_sinks.push(DangerSink {
                    kind: DangerSinkKind::MemberAccess,
                    target: base_vid,
                    range: self.range(node),
                    label: self.text(node),
                });
                self.emit(
                    Some(self.text(node)),
                    node,
                    SsiOp::ProjectField {
                        base: base_vid,
                        field,
                    },
                )
            }
            "index_expression" => {
                let base_vid = node
                    .child_by_field_name("operand")
                    .map(|o| self.lower_expr(o))
                    .unwrap_or_else(|| {
                        self.emit(
                            None,
                            node,
                            SsiOp::UnknownOp {
                                reason: UnknownReason::ExternalUntyped,
                                symbol: self.text(node),
                            },
                        )
                    });
                self.danger_sinks.push(DangerSink {
                    kind: DangerSinkKind::ElementIndex,
                    target: base_vid,
                    range: self.range(node),
                    label: self.text(node),
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
            "call_expression" => self.lower_call(node),
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

    fn lower_call(&mut self, node: Node<'_>) -> ValueId {
        let fn_node = node.child_by_field_name("function");
        let fn_text = fn_node.map(|f| self.text(f)).unwrap_or_default();

        let mut inputs = Vec::new();
        if let Some(args) = node.child_by_field_name("arguments") {
            let mut c = args.walk();
            for ch in args.named_children(&mut c) {
                inputs.push(self.lower_expr(ch));
            }
        }

        let target = if lookup_go_stdlib(&fn_text).is_some() {
            CallTarget::Stdlib(fn_text.clone())
        } else if let Some((pkg, sym)) = fn_text.split_once('.') {
            if let Some(imp) = self.imports.get(pkg) {
                CallTarget::Imported {
                    module_specifier: imp.module_specifier.clone(),
                    symbol: sym.to_string(),
                }
            } else if self.vars.contains_key(pkg) {
                CallTarget::Dynamic(fn_text.clone())
            } else {
                CallTarget::Local(fn_text.clone())
            }
        } else {
            CallTarget::Local(fn_text.clone())
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
            return AbsenceValue::bottom();
        };
        match &inst.op {
            SsiOp::Const { value, .. } => *value,
            SsiOp::Param {
                default_absence, ..
            } => *default_absence,
            SsiOp::Assign { src }
            | SsiOp::Pi { src, .. }
            | SsiOp::ProjectField { base: src, .. }
            | SsiOp::ProjectIndex { base: src, .. }
            | SsiOp::UnwrapVariant { base: src, .. } => self.approx_value_absence(*src),
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
            } => lookup_go_stdlib(sym).unwrap_or_else(AbsenceValue::present),
            SsiOp::UnknownOp { reason, .. } => AbsenceValue::unknown(*reason),
            _ => AbsenceValue::present(),
        }
    }

    fn finish(mut self, skeleton: &FunctionSkeleton) -> FunctionIr {
        self.span_index.sort_by_key(|(r, _)| (r.start, r.end));
        let param_flow_to_return = compute_param_flow_to_return(&self.insts, &self.returns);
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
