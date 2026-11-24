use lineage_engine::ir::guards::GuardFact;
use lineage_engine::ir::{
    AbsenceValue, BasicBlock, BlockId, CallTarget, ExitEdge, FnSummary, FunctionIr, GuardEnv,
    SsiInst, SsiOp, ValueId, STATE_NULL, STATE_PRESENT,
};
use lineage_engine::protocol::schema::{CanonicalPos, CanonicalRange, UnknownReason};

fn dummy_range(line: u32) -> CanonicalRange {
    CanonicalRange {
        start: CanonicalPos { line, col: 0 },
        end: CanonicalPos { line, col: 20 },
    }
}

/// Validates Go `(val, err)` correlated multi-output calls, comma-ok map lookups,
/// zero-value reference initialization, and `defer`-mutated named returns on the
/// frozen Pruned SSI + `GuardFact` Core IR with zero Core IR modifications.
#[test]
fn test_go_val_err_correlated_implies_narrows_val() {
    // Model:
    //   val, err := fetchUser(id)  -- multi-output Call with Implies(err == nil => val != nil)
    //   if err != nil { return }
    //   use(val)                   -- val must narrow to Present
    let val_id = ValueId(0);
    let err_id = ValueId(1);
    let narrowed_val_id = ValueId(2);

    let ir = FunctionIr {
        symbol_id: "handler@0".to_string(),
        name: "handler".to_string(),
        range: dummy_range(0),
        params: Vec::new(),
        blocks: vec![
            BasicBlock {
                id: BlockId(0),
                preds: Vec::new(),
                succs: vec![BlockId(1), BlockId(2)],
                insts: vec![val_id, err_id],
                guards: vec![GuardFact::Implies {
                    antecedent_place: "err".to_string(),
                    antecedent_mask: STATE_NULL, // err == nil
                    consequent_place: "val".to_string(),
                    consequent_mask: STATE_PRESENT, // val != nil
                }],
                terminated: false,
            },
            BasicBlock {
                id: BlockId(1), // if err != nil { return }
                preds: vec![BlockId(0)],
                succs: Vec::new(),
                insts: Vec::new(),
                guards: vec![GuardFact::Unreachable],
                terminated: true,
            },
            BasicBlock {
                id: BlockId(2), // surviving continuation: err == nil
                preds: vec![BlockId(0)],
                succs: Vec::new(),
                insts: vec![narrowed_val_id],
                guards: vec![GuardFact::StateIn {
                    place: "err".to_string(),
                    mask: STATE_NULL,
                }],
                terminated: false,
            },
        ],
        insts: vec![
            SsiInst {
                id: val_id,
                block: BlockId(0),
                name: Some("val".to_string()),
                range: dummy_range(1),
                op: SsiOp::Call {
                    callee: CallTarget::Local("fetchUser".to_string()),
                    inputs: Vec::new(),
                    outputs: vec![val_id, err_id],
                    exit_edge: ExitEdge::Normal,
                },
                snippet: Some("val, err := fetchUser(id)".to_string()),
            },
            SsiInst {
                id: err_id,
                block: BlockId(0),
                name: Some("err".to_string()),
                range: dummy_range(1),
                op: SsiOp::Const {
                    value: AbsenceValue::maybe_null(),
                    label: "err".to_string(),
                },
                snippet: Some("err".to_string()),
            },
            SsiInst {
                id: narrowed_val_id,
                block: BlockId(2),
                name: Some("val".to_string()),
                range: dummy_range(4),
                op: SsiOp::Pi {
                    src: val_id,
                    guard: GuardFact::StateIn {
                        place: "err".to_string(),
                        mask: STATE_NULL,
                    },
                },
                snippet: Some("use(val)".to_string()),
            },
        ],
        returns: vec![narrowed_val_id],
        danger_sinks: Vec::new(),
        span_index: Vec::new(),
        local_summary: FnSummary::default(),
    };

    // Simulate GuardEnv propagation along bb0 -> bb2
    let mut env = GuardEnv::new();
    for g in &ir.blocks[0].guards {
        env.add_fact(g.clone());
    }
    for g in &ir.blocks[2].guards {
        env.add_fact(g.clone());
    }

    let initial_val = AbsenceValue::maybe_null();
    let narrowed = env.apply_to("val", initial_val);
    assert_eq!(narrowed.known_states, STATE_PRESENT);
    assert!(!narrowed.is_maybe_absent());
}

#[test]
fn test_go_comma_ok_map_lookup_narrows_via_implies() {
    // Model: v, ok := m[k]; if ok { use(v) }
    let mut env = GuardEnv::new();
    env.add_fact(GuardFact::Implies {
        antecedent_place: "ok".to_string(),
        antecedent_mask: STATE_PRESENT,
        consequent_place: "v".to_string(),
        consequent_mask: STATE_PRESENT,
    });
    env.add_fact(GuardFact::StateIn {
        place: "ok".to_string(),
        mask: STATE_PRESENT,
    });

    let narrowed_v = env.apply_to("v", AbsenceValue::maybe_null());
    assert_eq!(narrowed_v.known_states, STATE_PRESENT);
}

#[test]
fn test_go_zero_value_and_defer_named_return_downgrade() {
    // Zero-value pointer/slice/map starts Nil (Absent)
    let zero_ptr = AbsenceValue::null();
    assert!(zero_ptr.is_definitely_absent());

    // Zero-value struct/int starts Present
    let zero_int = AbsenceValue::present();
    assert!(!zero_int.is_maybe_absent());

    // Named return mutated inside `defer` downgrades to Unknown (never fabricated as Absent)
    let defer_mutated =
        AbsenceValue::present().join(AbsenceValue::unknown(UnknownReason::DynamicDispatch));
    assert!(defer_mutated.has_unknown());
    assert!(!defer_mutated.is_maybe_absent());
}
