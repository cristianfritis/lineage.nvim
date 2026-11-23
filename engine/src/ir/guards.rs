use std::collections::HashMap;

use crate::ir::lattice::{AbsenceValue, STATE_PRESENT};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum GuardFact {
    StateIn {
        place: String,
        mask: u8,
    },
    Implies {
        antecedent_place: String,
        antecedent_mask: u8,
        consequent_place: String,
        consequent_mask: u8,
    },
    Unreachable,
}

#[derive(Debug, Clone, Default)]
pub struct GuardEnv {
    pub constraints: HashMap<String, u8>,
    pub implications: Vec<(String, u8, String, u8)>,
    pub unreachable: bool,
}

impl GuardEnv {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_fact(&mut self, fact: GuardFact) {
        match fact {
            GuardFact::Unreachable => {
                self.unreachable = true;
            }
            GuardFact::StateIn { place, mask } => {
                self.constraints
                    .entry(place)
                    .and_modify(|m| *m &= mask)
                    .or_insert(mask);
                self.propagate();
            }
            GuardFact::Implies {
                antecedent_place,
                antecedent_mask,
                consequent_place,
                consequent_mask,
            } => {
                self.implications.push((
                    antecedent_place,
                    antecedent_mask,
                    consequent_place,
                    consequent_mask,
                ));
                self.propagate();
            }
        }
    }

    /// Fixed-point propagation of bounded `Implies` rules (at most `implications.len() + 1` steps).
    fn propagate(&mut self) {
        for _ in 0..=self.implications.len() {
            let mut changed = false;
            for (ant_p, ant_m, cons_p, cons_m) in &self.implications {
                if let Some(&cur_ant) = self.constraints.get(ant_p) {
                    if cur_ant != 0 && (cur_ant & !ant_m) == 0 {
                        let prev = self.constraints.get(cons_p).copied().unwrap_or(0xFF);
                        let next = prev & *cons_m;
                        if next != prev {
                            self.constraints.insert(cons_p.clone(), next);
                            changed = true;
                        }
                    }
                }
            }
            if !changed {
                break;
            }
        }
    }

    pub fn apply_to(&self, place: &str, val: AbsenceValue) -> AbsenceValue {
        if self.unreachable {
            return AbsenceValue::bottom();
        }
        if let Some(&mask) = self.constraints.get(place) {
            let narrowed = val.narrow_mask(mask);
            // If a variable was Unknown (no known_states bits yet) and narrowed to Present,
            // record Present as the narrowed witness.
            if val.known_states == 0 && (mask & STATE_PRESENT) != 0 && mask == STATE_PRESENT {
                return AbsenceValue {
                    known_states: STATE_PRESENT,
                    unknown_reasons: val.unknown_reasons,
                };
            }
            return narrowed;
        }
        val
    }
}
